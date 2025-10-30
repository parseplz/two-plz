use std::sync::{Arc, Mutex};

use crate::{
    Frame,
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
}
