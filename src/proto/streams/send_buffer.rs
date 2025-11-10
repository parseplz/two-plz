use std::sync::Mutex;

use crate::{Frame, proto::streams::buffer::Buffer};

#[derive(Debug)]
pub struct SendBuffer<B> {
    pub inner: Mutex<Buffer<Frame<B>>>,
}

impl<B> Default for SendBuffer<B> {
    fn default() -> Self {
        Self::new()
    }
}

impl<B> SendBuffer<B> {
    pub fn new() -> Self {
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
