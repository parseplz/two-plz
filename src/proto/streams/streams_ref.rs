use std::sync::{Arc, Mutex};

use crate::{
    codec::UserError,
    message::{TwoTwoFrame, request::Request, response::Response},
    proto::streams::{
        inner::Inner,
        opaque_streams_ref::OpaqueStreamRef,
        send_buffer::SendBuffer,
        store::{Ptr, Resolve},
    },
    server::Server,
};

/// Reference to the stream state
#[derive(Debug)]
pub(crate) struct StreamRef<B> {
    pub opaque: OpaqueStreamRef,
    pub send_buffer: Arc<SendBuffer<B>>,
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
