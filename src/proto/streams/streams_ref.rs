use std::sync::{Arc, Mutex};

use bytes::Bytes;
use tracing::{debug, trace};

use crate::{
    codec::UserError,
    error::Reason,
    frame::Frame,
    message::{
        InfoLine, TwoTwo, TwoTwoFrame, request::Request, response::Response,
    },
    proto::{
        error::Initiator,
        streams::{
            buffer::Buffer,
            inner::Inner,
            opaque_streams_ref::OpaqueStreamRef,
            send_buffer::SendBuffer,
            store::{Key, Ptr, Resolve},
            streams::queue_body_trailer,
        },
    },
    server::Server,
};

/// Reference to the stream state
#[derive(Debug)]
pub(crate) struct StreamRef<B> {
    pub opaque: OpaqueStreamRef,
    pub send_buffer: Arc<SendBuffer<B>>,
}

impl StreamRef<Bytes> {
    pub fn new(
        inner: Arc<Mutex<Inner>>,
        ptr: &mut Ptr,
        send_buffer: Arc<SendBuffer<Bytes>>,
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

    pub fn send_response(
        &mut self,
        mut response: Response,
    ) -> Result<(), UserError> {
        let mut me = self.opaque.inner.lock().unwrap();
        let me = &mut *me;
        let mut stream = me.store.resolve(self.opaque.key);
        let span =
            tracing::debug_span!("[+] send response| ", "{:?}", stream.id);
        let _ = span.enter();
        let actions = &mut me.actions;
        let mut send_buffer = self.send_buffer.inner.lock().unwrap();
        let send_buffer = &mut *send_buffer;
        let mut response_frames = TwoTwoFrame::from((stream.id, response));
        let data_frame = response_frames.take_data();
        let trailer_frame = response_frames.take_trailer();

        me.counts
            .transition(stream, |counts, stream| {
                actions.send.send_headers(
                    response_frames.header,
                    send_buffer,
                    stream,
                    counts,
                    &mut actions.task,
                )
            });
        trace!("[+] added| header");

        let mut stream = me.store.resolve(self.opaque.key);

        queue_body_trailer(
            &mut stream,
            data_frame,
            trailer_frame,
            send_buffer,
        );

        trace!("[+] response queued");
        Ok(())
    }

    // TODO: expose API
    pub fn send_reset(&mut self, reason: Reason) {
        let mut me = self.opaque.inner.lock().unwrap();
        let me = &mut *me;

        let stream = me.store.resolve(self.opaque.key);
        let mut send_buffer = self.send_buffer.inner.lock().unwrap();
        let send_buffer = &mut *send_buffer;

        match me.actions.send_reset(
            stream,
            reason,
            Initiator::User,
            &mut me.counts,
            send_buffer,
        ) {
            Ok(()) => (),
            Err(crate::proto::error::GoAway {
                ..
            }) => {
                // this should never happen, because Initiator::User resets do
                // not count toward the local limit.
                // we could perhaps make this state impossible, if we made the
                // initiator argument a generic, and so this could return
                // Infallible instead of an impossible GoAway, but oh well.
                unreachable!("Initiator::User should not error sending reset");
            }
        }
    }
}

impl<B> Clone for StreamRef<B> {
    fn clone(&self) -> Self {
        StreamRef {
            opaque: self.opaque.clone(),
            send_buffer: self.send_buffer.clone(),
        }
    }
}
