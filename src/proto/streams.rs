use std::sync::{Arc, Mutex};

use crate::{
    Frame, Settings,
    proto::{
        ProtoError, buffer::Buffer, config::ConnectionConfig, count::Counts,
        recv::Recv, send::Send, store::Store,
    },
    role::Role,
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
            .apply_remote_settings(&settings);
        me.actions
            .send
            .apply_remote_settings(&settings, &mut me.store)
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
            counts: Counts::new(&config),
            actions: Actions::new(role, config),
            store: Store::new(),
            refs: 1,
        }))
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

    // ===== Misc =====

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

        if let Ok(next) = next_id {
            if id >= next {
                return Err(Reason::PROTOCOL_ERROR);
            }
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
    fn may_have_forgotten_stream(&self, role: Role, id: StreamId) -> bool {
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
