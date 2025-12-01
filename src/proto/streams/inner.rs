use crate::Codec;
use crate::frame;
use crate::frame::Reason;
use crate::frame::StreamId;
use crate::proto::MAX_WINDOW_SIZE;
use crate::proto::ProtoError;
use crate::proto::WindowSize;
use crate::proto::error::Initiator;
use crate::proto::streams::send_buffer::SendBuffer;
use crate::proto::{
    config::ConnectionConfig,
    streams::{
        Store,
        action::Actions,
        counts::Counts,
        recv::{Open, RecvHeaderBlockError},
        store::{Entry, Key, Resolve},
        stream::Stream,
    },
};
use std::io;
use std::sync::Arc;
use std::sync::Mutex;
use std::task::Context;
use std::task::Poll;

use crate::role::Role;
use bytes::Bytes;
use tokio::io::AsyncWrite;
use tracing::error;
use tracing::trace;

/// Fields needed to manage state related to managing the set of streams. This
/// is mostly split out to make ownership happy.
#[derive(Debug)]
pub struct Inner {
    /// Tracks send & recv stream concurrency.
    pub counts: Counts,

    /// Connection level state and performs actions on streams
    pub actions: Actions,

    /// Stores stream state
    pub store: Store,

    /// The number of stream refs to this shared state.
    pub refs: usize,
}

impl Inner {
    pub fn new(role: Role, config: ConnectionConfig) -> Arc<Mutex<Inner>> {
        Arc::new(Mutex::new(Inner {
            counts: Counts::new(role.clone(), &config),
            actions: Actions::new(role, config),
            store: Store::new(),
            refs: 1,
        }))
    }

    // ===== Data =====
    pub fn recv_data(
        &mut self,
        send_buffer: &SendBuffer<Bytes>,
        frame: frame::Data,
    ) -> Result<(), ProtoError> {
        let id = frame.stream_id();
        let stream = match self.store.find_mut(&id) {
            Some(stream) => stream,
            None => {
                // The GOAWAY process has begun. All streams with a greater ID
                // than specified as part of GOAWAY should be ignored.
                if id > self.actions.recv.max_stream_id() {
                    return Ok(());
                }

                if self
                    .actions
                    .is_forgotten_stream(&self.counts.role(), id)
                {
                    let sz = frame.payload().len();
                    // This should have been enforced at the codec::FramedRead
                    // layer, so this is just a sanity check.
                    assert!(sz <= MAX_WINDOW_SIZE as usize);
                    let sz = sz as WindowSize;
                    // consume connection flow control
                    self.actions
                        .recv
                        .dec_connection_window(sz)?;
                    return Err(ProtoError::library_reset(
                        id,
                        Reason::STREAM_CLOSED,
                    ));
                }

                return Err(ProtoError::library_go_away(
                    Reason::PROTOCOL_ERROR,
                ));
            }
        };

        let actions = &mut self.actions;
        let mut send_buffer = send_buffer.inner.lock().unwrap();
        let send_buffer = &mut *send_buffer;
        let role = self.counts.role();

        self.counts
            .transition(stream, |counts, stream| {
                let sz = frame.payload().len();
                let res = actions
                    .recv
                    .recv_data(frame, stream, &role);

                // Any stream error after receiving a DATA frame means
                // we won't give the data to the user, and so they can't
                // release the capacity. We do it automatically.
                if let Err(ProtoError::Reset(..)) = res {
                    actions
                        .recv
                        .dec_connection_window(sz as WindowSize)?;
                }
                actions.reset_on_recv_stream_err(
                    send_buffer,
                    stream,
                    counts,
                    res,
                )
            })
    }

    // ===== Headers =====
    pub fn recv_headers(
        &mut self,
        send_buffer: &SendBuffer<Bytes>,
        frame: frame::Headers,
    ) -> Result<(), ProtoError> {
        let id = frame.stream_id();
        let role = self.counts.role();

        // The GOAWAY process has begun. All streams with a greater ID than
        // specified as part of GOAWAY should be ignored.
        if id > self.actions.recv.max_stream_id() {
            return Ok(());
        }

        // Insert stream in store
        let key = match self.insert_or_create(id, &role)? {
            Some(key) => key,
            None => return Ok(()),
        };

        let stream = self.store.resolve(key);

        if stream.state.is_local_error() {
            // Locally reset streams must ignore frames "for some time".
            // This is because the remote may have sent trailers before
            // receiving the RST_STREAM frame.
            trace!("recv_headers| ignoring trailers on|{:?}", stream.id);
            return Ok(());
        }

        let actions = &mut self.actions;
        let mut send_buffer = send_buffer.inner.lock().unwrap();
        let send_buffer = &mut *send_buffer;

        self.counts.transition(stream, |counts, stream| {
            let res = if stream.state.is_recv_headers() {
                match actions.recv.recv_headers(frame, stream, counts) {
                    Ok(()) => Ok(()),
                    Err(RecvHeaderBlockError::Oversize(resp)) => {
                        // server => 431 error 
                        if let Some(resp) = resp {
                            let sent = actions.send.send_headers(
                                resp, send_buffer, stream, counts, &mut actions.task);
                            debug_assert!(sent.is_ok(), "oversize response should not fail");
                            actions.send.schedule_implicit_reset(
                                stream,
                                Reason::PROTOCOL_ERROR,
                                counts,
                                &mut actions.task);
                            actions.recv.enqueue_reset_expiration(stream, counts);
                            Ok(())
                        } else {
                            // client => ProtoError
                            Err(ProtoError::library_reset(stream.id, Reason::PROTOCOL_ERROR))
                        }
                    },
                    Err(RecvHeaderBlockError::State(e)) => Err(e),
                }
            } else {
                // Trailers
                if !frame.is_end_stream() {
                    // Receiving trailers that don't set EOS is a "malformed"
                    // message. Malformed messages are a stream error.
                    proto_err!(stream: "trailers withour EOS| stream={:?}", stream.id);
                    return Err(ProtoError::library_reset(stream.id, Reason::PROTOCOL_ERROR));
                }
                actions.recv.recv_trailers(frame, stream, &role)
            };
            actions.reset_on_recv_stream_err(send_buffer, stream, counts, res)
        })
    }

    #[inline(always)]
    fn insert_or_create(
        &mut self,
        id: StreamId,
        role: &Role,
    ) -> Result<Option<Key>, ProtoError> {
        let key = match self.store.find_entry(id) {
            Entry::Occupied(entry) => {
                trace!("new entry| {:?}", id);
                entry.key()
            }
            Entry::Vacant(entry) => {
                // Client: it's possible to send a request, and then send
                // a RST_STREAM while the response HEADERS were in transit.
                //
                // Server: we can't reset a stream before having received
                // the request headers, so don't allow.
                if !role.is_server() {
                    // This may be response headers for a stream we've already
                    // forgotten about...
                    if self
                        .actions
                        .is_forgotten_stream(role, id)
                    {
                        error!("insert_or_create| forgotten stream| {:?}", id);
                        return Err(ProtoError::library_reset(
                            id,
                            Reason::STREAM_CLOSED,
                        ));
                    }
                }

                // check if the stream Id is the exepected and within the limit
                // of total no of recv streams
                match self.actions.recv.can_open(
                    id,
                    Open::Headers,
                    &mut self.counts,
                    role,
                )? {
                    Some(stream_id) => {
                        let stream = Stream::new(
                            stream_id,
                            self.actions.send.init_window_sz(),
                            self.actions.recv.init_window_sz(),
                        );
                        entry.insert(stream)
                    }
                    None => return Ok(None),
                }
            }
        };
        Ok(Some(key))
    }

    // ===== Reset =====
    pub fn recv_reset<B>(
        &mut self,
        send_buffer: &SendBuffer<B>,
        frame: frame::Reset,
    ) -> Result<(), ProtoError> {
        let id = frame.stream_id();
        if id.is_zero() {
            proto_err!(conn: "recv_reset| invalid stream ID 0");
            return Err(ProtoError::library_go_away(Reason::PROTOCOL_ERROR));
        }

        // The GOAWAY process has begun. All streams with a greater ID than
        // specified as part of GOAWAY should be ignored.
        if id > self.actions.recv.max_stream_id() {
            return Ok(());
        }

        let stream = match self.store.find_mut(&id) {
            Some(stream) => stream,
            None => {
                // TODO: Are there other error cases?
                self.actions
                    .ensure_not_idle(self.counts.role(), id)
                    .map_err(ProtoError::library_go_away)?;
                return Ok(());
            }
        };

        let mut send_buffer = send_buffer.inner.lock().unwrap();
        let send_buffer = &mut *send_buffer;
        let actions = &mut self.actions;

        self.counts
            .transition(stream, |counts, stream| {
                actions
                    .recv
                    .recv_reset(frame, stream, counts)?;
                actions
                    .send
                    .handle_error(send_buffer, stream, counts);
                assert!(stream.state.is_closed());
                Ok(())
            })
    }

    pub fn send_reset(
        &mut self,
        send_buffer: &SendBuffer<Bytes>,
        id: StreamId,
        reason: Reason,
    ) -> Result<(), crate::proto::error::GoAway> {
        let key = match self.store.find_entry(id) {
            Entry::Occupied(e) => e.key(),
            Entry::Vacant(e) => {
                // Resetting a stream we don't know about? That could be OK...
                //
                // 1. As a server, we just received a request, but that request
                //    was bad, so we're resetting before even accepting it.
                //    This is totally fine.
                //
                // 2. The remote may have sent us a frame on new stream that
                //    it's *not* supposed to have done, and thus, we don't know
                //    the stream. In that case, sending a reset will "open" the
                //    stream in our store. Maybe that should be a connection
                //    error instead? At least for now, we need to update what
                //    our vision of the next stream is.
                if self.counts.role.is_local_init(id) {
                    // We normally would open this stream, so update our
                    // next-send-id record.
                    self.actions
                        .send
                        .maybe_reset_next_stream_id(id);
                } else {
                    // We normally would recv this stream, so update our
                    // next-recv-id record.
                    self.actions
                        .recv
                        .maybe_reset_next_stream_id(id);
                }

                let stream = Stream::new(id, 0, 0);
                e.insert(stream)
            }
        };

        let stream = self.store.resolve(key);
        let mut send_buffer = send_buffer.inner.lock().unwrap();
        let send_buffer = &mut *send_buffer;
        self.actions.send_reset(
            stream,
            reason,
            Initiator::Library,
            &mut self.counts,
            send_buffer,
        )
    }

    // ===== Goaway =====
    pub fn recv_go_away(
        &mut self,
        send_buffer: &SendBuffer<Bytes>,
        frame: &frame::GoAway,
    ) -> Result<(), ProtoError> {
        let actions = &mut self.actions;
        let counts = &mut self.counts;
        let mut send_buffer = send_buffer.inner.lock().unwrap();
        let send_buffer = &mut *send_buffer;

        let last_stream_id = frame.last_stream_id();

        actions
            .send
            .recv_go_away(last_stream_id)?;

        let err = ProtoError::remote_go_away(
            frame.debug_data().clone(),
            frame.reason(),
        );

        self.store.for_each(|stream| {
            if stream.id > last_stream_id {
                counts.transition(stream, |counts, stream| {
                    // notify receivers
                    actions
                        .recv
                        .handle_error(&err, &mut *stream);
                    // clear pending buffer
                    actions
                        .send
                        .handle_error(send_buffer, stream, counts);
                })
            }
        });

        actions.conn_error = Some(err);

        Ok(())
    }

    pub fn handle_error<B>(
        &mut self,
        send_buffer: &SendBuffer<B>,
        err: ProtoError,
    ) -> StreamId {
        let actions = &mut self.actions;
        let counts = &mut self.counts;
        let mut send_buffer = send_buffer.inner.lock().unwrap();
        let send_buffer = &mut *send_buffer;

        let last_processed_id = actions.recv.last_processed_id();

        self.store.for_each(|stream| {
            counts.transition(stream, |counts, stream| {
                // notify receivers
                actions
                    .recv
                    .handle_error(&err, &mut *stream);
                // clear pending buffer
                // TODO: should reclaim capacity ?
                actions
                    .send
                    .handle_error(send_buffer, stream, counts);
            })
        });

        actions.conn_error = Some(err);

        last_processed_id
    }

    // ===== window update =====
    pub fn poll_window_update<T>(
        &mut self,
        cx: &mut Context,
        dst: &mut Codec<T, Bytes>,
    ) -> Poll<std::io::Result<()>>
    where
        T: AsyncWrite + Unpin,
    {
        if !self
            .actions
            .recv
            .check_connection_window_update
        {
            return Poll::Ready(Ok(()));
        }

        if let Some(size) = self.should_send_connection_window_update() {
            ready!(dst.poll_ready(cx))?;
            let frame = frame::WindowUpdate::new(StreamId::ZERO, size);
            dst.buffer(frame.into())
                .expect("invalid WINDOW_UPDATE frame");
            let _ = self
                .actions
                .recv
                .inc_connection_window(size);
        }

        if let Some((stream_id, size)) =
            self.should_send_stream_window_update()
        {
            ready!(dst.poll_ready(cx))?;
            let frame = frame::WindowUpdate::new(stream_id, size);
            dst.buffer(frame.into())
                .expect("invalid WINDOW_UPDATE frame");
            let mut stream = self.store.find_mut(&stream_id).unwrap();
            let _ = stream.recv_flow.inc_window(size);
        }

        self.actions
            .recv
            .check_connection_window_update = false;
        self.actions
            .recv
            .check_stream_window_update = None;

        Poll::Ready(Ok(()))
    }

    pub fn should_send_connection_window_update(
        &mut self,
    ) -> Option<WindowSize> {
        self.actions
            .recv
            .should_send_connection_window_update()
            .map(|s| s as WindowSize)
    }

    pub fn should_send_stream_window_update(
        &mut self,
    ) -> Option<(StreamId, WindowSize)> {
        self.actions
            .recv
            .check_stream_window_update
            .as_ref()
            .and_then(|key| {
                self.store[*key]
                    .recv_flow
                    .should_send_window_update()
                    .map(|size| (self.store[*key].id, size as WindowSize))
            })
    }

    pub fn poll_complete<T>(
        &mut self,
        send_buffer: &SendBuffer<Bytes>,
        cx: &mut Context,
        dst: &mut Codec<T, Bytes>,
    ) -> Poll<std::io::Result<()>>
    where
        T: AsyncWrite + Unpin,
    {
        let mut send_buffer = send_buffer.inner.lock().unwrap();
        let send_buffer = &mut *send_buffer;
        trace!("[+] polling streams");
        ready!(self.actions.send.poll_complete(
            cx,
            send_buffer,
            &mut self.store,
            &mut self.counts,
            dst
        ))?;

        // Nothing else to do, track the task
        self.actions.task = Some(cx.waker().clone());
        Poll::Ready(Ok(()))
    }

    // ===== EOF =====
    pub fn recv_eof<B>(
        &mut self,
        send_buffer: &SendBuffer<B>,
        clear_pending_accept: bool,
    ) -> Result<(), ()> {
        let actions = &mut self.actions;
        let counts = &mut self.counts;
        let mut send_buffer = send_buffer.inner.lock().unwrap();
        let send_buffer = &mut *send_buffer;
        if actions.conn_error.is_none() {
            actions.conn_error = Some(
                io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "connection closed because of a broken pipe",
                )
                .into(),
            );
        }

        self.store.for_each(|stream| {
            counts.transition(stream, |counts, stream| {
                // notify
                actions.recv.recv_eof(stream);

                // clear queues and reclaim capacity
                actions
                    .send
                    .handle_error(send_buffer, stream, counts);
            })
        });

        // clear send and recv queues
        actions.clear_queues(clear_pending_accept, &mut self.store, counts);
        Ok(())
    }
}
