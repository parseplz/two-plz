use std::sync::{Arc, Mutex};

use crate::{
    message::request::Request,
    proto::streams::{
        Resolve, inner::Inner, opaque_streams_ref::OpaqueStreamRef,
        send_buffer::SendBuffer, store::Ptr,
    },
};

/// Reference to the stream state
#[derive(Debug)]
pub(crate) struct StreamRef<B> {
    opaque: OpaqueStreamRef,
    send_buffer: Arc<SendBuffer<B>>,
}

impl<B> StreamRef<B> {
    pub fn new(
        inner: Arc<Mutex<Inner>>,
        ptr: &mut Ptr,
        send_buffer: Arc<SendBuffer<B>>,
    ) -> Self {
        StreamRef {
            opaque: OpaqueStreamRef::new(inner, ptr),
            send_buffer,
        }
    }

    pub fn take_request(&self) -> Request {
        let mut me = self.opaque.inner.lock().unwrap();
        let me = &mut *me;
        let mut stream = me.store.resolve(self.opaque.key);
        me.actions
            .recv
            .take_request(&mut stream)
    }
}
