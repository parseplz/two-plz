use bytes::{Buf, Bytes};
use tokio::io::AsyncWrite;
use tracing::{trace, trace_span};

use crate::{
    Codec,
    client::ResponseFuture,
    codec::{SendError, UserError},
    frame::{self, Frame},
    message::{TwoTwoFrame, request::Request},
    proto::{
        WindowSize,
        config::ConnectionConfig,
        error::Initiator,
        streams::{
            buffer::Buffer,
            inner::Inner,
            opaque_streams_ref::OpaqueStreamRef,
            send_buffer::SendBuffer,
            store::{Ptr, Resolve},
            stream::Stream,
            streams_ref::StreamRef,
        },
    },
};

use std::{
    sync::{Arc, Mutex},
    task::{Context, Poll},
};

use crate::frame::{Reason, Settings, StreamId};
use crate::{proto::ProtoError, role::Role};

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

impl Streams<Bytes> {
    pub fn new(role: Role, config: ConnectionConfig) -> Self {
        Streams {
            inner: Inner::new(role, config),
            send_buffer: Arc::new(SendBuffer::new()),
        }
    }

    pub fn send_request(
        &mut self,
        mut request: Request,
    ) -> Result<OpaqueStreamRef, SendError> {
        use super::stream::ContentLength;
        use http::Method;
        // TODO: why ?
        //let protocol = request
        //    .extensions_mut()
        //    .remove::<Protocol>();
        //request.extensions_mut().clear();

        // TODO: There is a hazard with assigning a stream ID before the
        // prioritize layer. If prioritization reorders new streams, this
        // implicitly closes the earlier stream IDs.
        //
        // See: hyperium/h2#11

        let mut me = self.inner.lock().unwrap();
        let me = &mut *me;
        let mut send_buffer = self.send_buffer.inner.lock().unwrap();
        let send_buffer = &mut *send_buffer;
        me.actions.ensure_no_conn_error()?;
        me.actions
            .send
            .ensure_next_stream_id()?;

        if me.counts.role().is_server() {
            // Servers cannot open streams. PushPromise must first be reserved.
            return Err(UserError::UnexpectedFrameType.into());
        }

        let stream_id = me.actions.send.open()?;

        let mut stream = Stream::new(
            stream_id,
            me.actions.send.init_window_sz(),
            me.actions.recv.init_window_sz(),
        );

        if *request.method() == Method::HEAD {
            stream.content_length = ContentLength::Head;
        }

        let mut stream = me.store.insert(stream.id, stream);

        let mut request_frames = TwoTwoFrame::from((stream.id, request));
        let data_frame = request_frames.take_data();
        let trailer_frame = request_frames.take_trailer();

        // TODO: Error handling needed ?
        if let Err(e) = me.actions.send.send_headers(
            request_frames.header,
            send_buffer,
            &mut stream,
            &mut me.counts,
            &mut me.actions.task,
        ) {
            stream.unlink();
            stream.remove();
            return Err(e.into());
        }
        queue_body_trailer(
            &mut stream,
            data_frame,
            trailer_frame,
            send_buffer,
        );

        // Given that the stream has been initialized, it should not be in the
        // closed state.
        debug_assert!(!stream.state.is_closed());

        // TODO: ideally, OpaqueStreamRefs::new would do this, but we're
        // holding the lock, so it can't.
        me.refs += 1;

        Ok(OpaqueStreamRef::new(self.inner.clone(), &mut stream))
    }
    // ===== send =====

    pub fn next_accept(&mut self) -> Option<StreamRef<Bytes>> {
        let mut me = self.inner.lock().unwrap();
        let me = &mut *me;
        me.actions
            .recv
            .next_accept(&mut me.store)
            .map(|key| {
                let stream = &mut me.store.resolve(key);
                tracing::trace!(
                    "next_incoming| id={:?}, state={:?}",
                    stream.id,
                    stream.state
                );
                // TODO: ideally, OpaqueStreamRefs::new would do this, but
                // we're holding the lock, so it can't.
                me.refs += 1;

                // Pending-accepted remotely-reset streams are counted.
                if stream.state.is_remote_reset() {
                    me.counts.dec_num_remote_reset_streams();
                }

                StreamRef::new(
                    self.inner.clone(),
                    stream,
                    self.send_buffer.clone(),
                )
            })
    }

    // ===== Data =====
    pub fn recv_data(&mut self, frame: frame::Data) -> Result<(), ProtoError> {
        let mut me = self.inner.lock().unwrap();
        me.recv_data(&self.send_buffer, frame)
    }

    // ===== Header =====
    pub fn recv_header(
        &mut self,
        frame: frame::Headers,
    ) -> Result<(), ProtoError> {
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

        let mut send_buffer = self.send_buffer.inner.lock().unwrap();
        let send_buffer = &mut *send_buffer;

        me.counts
            .apply_remote_settings(settings);
        me.actions.send.apply_remote_settings(
            settings,
            send_buffer,
            &mut me.store,
            &mut me.counts,
            &mut me.actions.task,
        )
    }

    // ===== Goaway ====
    pub fn recv_go_away(
        &mut self,
        frame: &frame::GoAway,
    ) -> Result<(), ProtoError> {
        let mut me = self.inner.lock().unwrap();
        me.recv_go_away(&self.send_buffer, frame)
    }

    pub fn send_go_away(&mut self, last_processed_id: StreamId) {
        let mut me = self.inner.lock().unwrap();
        me.actions
            .recv
            .go_away(last_processed_id);
    }

    // ===== Reset =====
    pub fn recv_reset(
        &mut self,
        frame: frame::Reset,
    ) -> Result<(), ProtoError> {
        let mut me = self.inner.lock().unwrap();
        me.recv_reset(&self.send_buffer, frame)
    }

    pub fn send_reset(
        &mut self,
        id: StreamId,
        reason: Reason,
    ) -> Result<(), crate::proto::error::GoAway> {
        let mut me = self.inner.lock().unwrap();
        me.send_reset(&self.send_buffer, id, reason)
    }

    // ===== Window Update =====
    pub fn poll_window_update<T>(
        &mut self,
        cx: &mut Context,
        dst: &mut Codec<T, Bytes>,
    ) -> Poll<std::io::Result<()>>
    where
        T: AsyncWrite + Unpin,
    {
        let mut me = self.inner.lock().unwrap();
        me.poll_window_update(cx, dst)
    }

    pub fn recv_connection_window_update(
        &mut self,
        size: u32,
    ) -> Result<(), ProtoError> {
        let mut me = self.inner.lock().unwrap();
        let me = &mut *me;
        me.actions
            .send
            .recv_connection_window_update(size, &mut me.store, &mut me.counts)
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
                let mut send_buffer = self.send_buffer.inner.lock().unwrap();
                let send_buffer = &mut *send_buffer;
                // send reset
                me.actions.send.send_reset(
                    Reason::FLOW_CONTROL_ERROR,
                    Initiator::Library,
                    &mut stream,
                    send_buffer,
                    &mut me.counts,
                    &mut me.actions.task,
                )
            }
            Ok(())
        } else {
            me.actions
                .ensure_not_idle(me.counts.role(), id)
                .map_err(ProtoError::library_go_away)
        }
    }

    // ===== clear =====
    pub fn clear_expired_reset_streams(&mut self) {
        let mut me = self.inner.lock().unwrap();
        let me = &mut *me;
        me.actions
            .recv
            .clear_expired_reset_streams(&mut me.store, &mut me.counts);
    }

    // ===== polling =====
    pub fn poll_complete<T>(
        &mut self,
        cx: &mut Context,
        dst: &mut Codec<T, Bytes>,
    ) -> Poll<std::io::Result<()>>
    where
        T: AsyncWrite + Unpin,
    {
        let mut me = self.inner.lock().unwrap();
        me.poll_complete(&self.send_buffer, cx, dst)
    }

    // ===== misc =====
    pub fn clear_expired_reset_streams(&mut self) {
        let mut me = self.inner.lock().unwrap();
        let me = &mut *me;
        me.actions
            .recv
            .clear_expired_reset_streams(&mut me.store, &mut me.counts);
    }

    pub fn has_streams(&self) -> bool {
        let me = self.inner.lock().unwrap();
        me.counts.has_streams()
    }

    pub fn has_streams_or_other_references(&self) -> bool {
        let me = self.inner.lock().unwrap();
        me.counts.has_streams() || me.refs > 1
    }

    pub fn last_processed_id(&self) -> StreamId {
        self.inner
            .lock()
            .unwrap()
            .actions
            .recv
            .last_processed_id()
    }

    pub fn send_buffer_is_empty(&self) -> bool {
        self.send_buffer.is_empty()
    }

    pub fn send_pending_refusal<T>(
        &mut self,
        cx: &mut Context,
        dst: &mut Codec<T, Bytes>,
    ) -> Poll<std::io::Result<()>>
    where
        T: AsyncWrite + Unpin,
    {
        let mut me = self.inner.lock().unwrap();
        let me = &mut *me;
        me.actions
            .recv
            .send_pending_refusal(cx, dst)
    }

    /// Notify all streams that a connection-level error happened.
    pub fn handle_error(&self, e: ProtoError) -> StreamId {
        let mut me = self.inner.lock().unwrap();
        me.handle_error(&self.send_buffer, e)
    }

    // ===== EOF =====
    pub fn recv_eof(&mut self, clear_pending_accept: bool) -> Result<(), ()> {
        let mut me = self.inner.lock().map_err(|_| ())?;
        me.recv_eof(&self.send_buffer, clear_pending_accept)
    }
}

impl<B> Clone for Streams<B> {
    fn clone(&self) -> Self {
        self.inner.lock().unwrap().refs += 1;
        Streams {
            inner: self.inner.clone(),
            send_buffer: self.send_buffer.clone(),
            //_p: ::std::marker::PhantomData,
        }
    }
}

impl<B> Drop for Streams<B> {
    fn drop(&mut self) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.refs -= 1;
            if inner.refs == 1 {
                if let Some(task) = inner.actions.task.take() {
                    task.wake();
                }
            }
        }
    }
}

pub fn queue_body_trailer(
    stream: &mut Ptr,
    data_frame: Option<frame::Data<Bytes>>,
    trailer_frame: Option<frame::Headers>,
    send_buffer: &mut Buffer<Frame>,
) {
    let set_closed = data_frame.is_some() || trailer_frame.is_some();
    if let Some(frame) = data_frame {
        trace!("[+] added| data");
        stream.remaining_data_len = Some(frame.payload().len());
        stream
            .pending_send
            .push_back(send_buffer, frame.into());
    }

    if let Some(frame) = trailer_frame {
        trace!("[+] added| trailer");
        stream
            .pending_send
            .push_back(send_buffer, frame.into());
    }

    if set_closed {
        stream.state.send_close();
    }
}
