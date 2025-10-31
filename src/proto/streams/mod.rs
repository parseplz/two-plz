mod buffer;
mod counts;
mod flow_control;
pub mod recv;
pub mod send;
use send::Send;
mod state;
mod store;
mod stream;
use crate::proto::{
    config::ConnectionConfig,
    error::Initiator,
    streams::{
        counts::Counts,
        recv::{Open, Recv, RecvHeaderBlockError},
        store::{Entry, Ptr, Resolve},
        stream::Stream,
    },
};
use buffer::Buffer;
use store::Store;
use tracing::trace;

use std::sync::{Arc, Mutex};

use crate::{
    Frame, Headers, Reason, Settings, StreamId, proto::ProtoError, role::Role,
};

#[derive(Debug)]
struct SendBuffer<B> {
    inner: Mutex<Buffer<Frame<B>>>,
}

impl<B> SendBuffer<B> {
    fn new() -> Self {
        let inner = Mutex::new(Buffer::new());
        SendBuffer {
            inner,
        }
    }

    pub fn is_empty(&self) -> bool {
        let buf = self.inner.lock().unwrap();
        buf.is_empty()
    }
}

#[derive(Debug)]
pub(crate) struct Streams<B> {
    /// Holds most of the connection and stream related state for processing
    /// HTTP/2 frames associated with streams.
    inner: Arc<Mutex<Inner>>,

    /// This is the queue of frames to be written to the wire. This is split out
    /// to avoid requiring a `B` generic on all public API types even if `B` is
    /// not technically required.
    ///
    /// Currently, splitting this out requires a second `Arc` + `Mutex`.
    /// However, it should be possible to avoid this duplication with a little
    /// bit of unsafe code. This optimization has been postponed until it has
    /// been shown to be necessary.
    send_buffer: Arc<SendBuffer<B>>,
    // TODO: Why ?
    //_p: ::std::marker::PhantomData<P>,
}

impl<B> Streams<B> {
    pub fn new(role: Role, config: ConnectionConfig) -> Self {
        Streams {
            inner: Inner::new(role, config),
            send_buffer: Arc::new(SendBuffer::new()),
        }
    }

    // ===== Header =====
    pub fn recv_header(&mut self, frame: Headers) -> Result<(), ProtoError> {
        let mut me = self.inner.lock().unwrap();
        me.recv_headers(&self.send_buffer, frame)
    }

    // ===== Settings =====
    pub fn apply_local_settings(
        &mut self,
        settings: &Settings,
    ) -> Result<(), ProtoError> {
        let mut me = self.inner.lock().unwrap();
        let me = &mut *me;

        me.actions
            .recv
            .apply_local_settings(settings, &mut me.store)
    }

    pub fn apply_remote_settings(
        &mut self,
        settings: &Settings,
    ) -> Result<(), ProtoError> {
        let mut me = self.inner.lock().unwrap();
        let me = &mut *me;
        me.counts
            .apply_remote_settings(settings);
        me.actions
            .send
            .apply_remote_settings(settings, &mut me.store)
    }

    // ==== Window Update =====
    pub fn recv_connection_window_update(
        &mut self,
        size: u32,
    ) -> Result<(), ProtoError> {
        let mut me = self.inner.lock().unwrap();
        let me = &mut *me;
        me.actions
            .send
            .recv_connection_window_update(size)
            .map_err(ProtoError::library_go_away)
    }

    pub fn recv_stream_window_update(
        &mut self,
        id: StreamId,
        size: u32,
    ) -> Result<(), ProtoError> {
        let mut me = self.inner.lock().unwrap();
        let me = &mut *me;
        if let Some(mut stream) = me.store.find_mut(&id) {
            if let Err(e) = me
                .actions
                .send
                .recv_stream_window_update(&mut stream, size)
            {
                // send reset
                me.actions.send.send_reset(
                    Reason::FLOW_CONTROL_ERROR,
                    Initiator::Library,
                    &mut stream,
                )
            }
            Ok(())
        } else {
            me.actions
                .ensure_not_idle(me.counts.role(), id)
                .map_err(ProtoError::library_go_away)
        }
    }

    // ===== Misc =====
}

/// Fields needed to manage state related to managing the set of streams. This
/// is mostly split out to make ownership happy.
#[derive(Debug)]
struct Inner {
    /// Tracks send & recv stream concurrency.
    counts: Counts,

    /// Connection level state and performs actions on streams
    actions: Actions,

    /// Stores stream state
    store: Store,

    /// The number of stream refs to this shared state.
    refs: usize,
}

impl Inner {
    fn new(role: Role, config: ConnectionConfig) -> Arc<Mutex<Inner>> {
        Arc::new(Mutex::new(Inner {
            counts: Counts::new(role.clone(), &config),
            actions: Actions::new(role, config),
            store: Store::new(),
            refs: 1,
        }))
    }

    fn recv_headers<B>(
        &mut self,
        send_buffer: &SendBuffer<B>,
        frame: Headers,
    ) -> Result<(), ProtoError> {
        let id = frame.stream_id();
        let role = self.counts.role();

        // The GOAWAY process has begun. All streams with a greater ID than
        // specified as part of GOAWAY should be ignored.
        if id > self.actions.recv.max_stream_id() {
            return Ok(());
        }

        let key = match self.store.find_entry(id) {
            Entry::Occupied(e) => e.key(),
            Entry::Vacant(e) => {
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
                        .is_forgotten_stream(&role, id)
                    {
                        return Err(ProtoError::library_reset(
                            id,
                            Reason::STREAM_CLOSED,
                        ));
                    }
                }

                match self.actions.recv.open(
                    id,
                    Open::Headers,
                    &mut self.counts,
                    &role,
                )? {
                    Some(stream_id) => {
                        let stream = Stream::new(
                            stream_id,
                            self.actions.send.init_window_sz(),
                            self.actions.recv.init_window_sz(),
                        );

                        e.insert(stream)
                    }
                    None => return Ok(()),
                }
            }
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
                        if let Some(header) = resp {
                            let sent = actions.send.send_headers(
                                header, send_buffer, stream, counts);
                            debug_assert!(sent.is_ok(), "oversize response should not fail");

                            actions.send.schedule_implicit_reset(
                                stream,
                                Reason::PROTOCOL_ERROR,
                                counts,
                            );

                            actions.recv.enqueue_reset_expiration(stream, counts);

                            Ok(())
                        } else {
                            Err(ProtoError::library_reset(stream.id, Reason::PROTOCOL_ERROR))
                        }
                    },
                    Err(RecvHeaderBlockError::State(err)) => Err(err),
                }
            } else {
                /// Trailers
                if !frame.is_end_stream() {
                    // Receiving trailers that don't set EOS is a "malformed"
                    // message. Malformed messages are a stream error.
                    proto_err!(stream: "recv_headers: trailers frame was not EOS; stream={:?}", stream.id);
                    return Err(ProtoError::library_reset(stream.id, Reason::PROTOCOL_ERROR));
                }

                actions.recv.recv_trailers(frame, stream)
            };
            actions.reset_on_recv_stream_err(send_buffer, stream, counts, res)
        })
    }
}

#[derive(Debug)]
struct Actions {
    /// Manages state transitions initiated by receiving frames
    recv: Recv,

    /// Manages state transitions initiated by sending frames
    send: Send,

    /// If the connection errors, a copy is kept for any StreamRefs.
    conn_error: Option<ProtoError>,
}

impl Actions {
    fn new(role: Role, config: ConnectionConfig) -> Self {
        Actions {
            recv: Recv::new(&config, &role),
            send: Send::new(&config, &role),
            conn_error: None,
        }
    }

    fn reset_on_recv_stream_err<B>(
        &mut self,
        buffer: &mut Buffer<Frame<B>>,
        stream: &mut Ptr,
        counts: &mut Counts,
        res: Result<(), ProtoError>,
    ) -> Result<(), ProtoError> {
        if let Err(ProtoError::Reset(stream_id, reason, initiator)) = res {
            debug_assert_eq!(stream_id, stream.id);

            if counts.can_inc_num_local_error_resets() {
                counts.inc_num_local_error_resets();
                // Reset the stream.
                self.send
                    .send_reset(reason, initiator, stream);
                self.recv
                    .enqueue_reset_expiration(stream, counts);
                Ok(())
            } else {
                Err(ProtoError::library_go_away_data(
                    Reason::ENHANCE_YOUR_CALM,
                    "too_many_internal_resets",
                ))
            }
        } else {
            res
        }
    }

    /// Check whether the stream was present in the past
    fn ensure_not_idle(
        &mut self,
        role: Role,
        id: StreamId,
    ) -> Result<(), Reason> {
        let next_id = if role.is_local_init(id) {
            self.send.next_stream_id
        } else {
            self.recv.next_stream_id
        };

        if let Ok(next) = next_id
            && id >= next {
                return Err(Reason::PROTOCOL_ERROR);
            }
        Ok(())
    }

    /// Check if we possibly could have processed and since forgotten this stream.
    ///
    /// If we send a RST_STREAM for a stream, we will eventually "forget" about
    /// the stream to free up memory. It's possible that the remote peer had
    /// frames in-flight, and by the time we receive them, our own state is
    /// gone. We *could* tear everything down by sending a GOAWAY, but it
    /// is more likely to be latency/memory constraints that caused this,
    /// and not a bad actor. So be less catastrophic, the spec allows
    /// us to send another RST_STREAM of STREAM_CLOSED.
    fn is_forgotten_stream(&self, role: &Role, id: StreamId) -> bool {
        if id.is_zero() {
            return false;
        }

        let next = if role.is_local_init(id) {
            self.send.next_stream_id
        } else {
            self.recv.next_stream_id
        };

        if let Ok(next_id) = next {
            debug_assert_eq!(
                id.is_server_initiated(),
                next_id.is_server_initiated(),
            );
            id < next_id
        } else {
            true
        }
    }
}
