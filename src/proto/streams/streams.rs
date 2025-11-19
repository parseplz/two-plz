use bytes::{Buf, Bytes};
use tokio::io::AsyncWrite;

use crate::{
    Codec, WindowUpdate, frame,
    proto::{
        WindowSize,
        config::ConnectionConfig,
        error::Initiator,
        streams::{
            inner::Inner, send_buffer::SendBuffer, store::Resolve,
            streams_ref::StreamRef,
        },
    },
};

use std::{
    sync::{Arc, Mutex},
    task::{Context, Poll},
};

use crate::{Reason, Settings, StreamId, proto::ProtoError, role::Role};

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
        me.counts
            .apply_remote_settings(settings);
        me.actions
            .send
            .apply_remote_settings(settings, &mut me.store)
    }

    // ===== Reset =====
    pub fn recv_reset(
        &mut self,
        frame: frame::Reset,
    ) -> Result<(), ProtoError> {
        let mut me = self.inner.lock().unwrap();
        me.recv_reset(&self.send_buffer, frame)
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
}
