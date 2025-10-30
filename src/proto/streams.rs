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
