use std::sync::{Arc, Mutex};

use bytes::Bytes;
use tracing::{debug, trace};

use crate::{
    codec::UserError,
    frame::Frame,
    message::{
        InfoLine, TwoTwo, TwoTwoFrame, request::Request, response::Response,
    },
    proto::streams::{
        buffer::Buffer,
        inner::Inner,
        opaque_streams_ref::OpaqueStreamRef,
        send_buffer::SendBuffer,
        store::{Key, Ptr, Resolve},
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
        stream.state.send_close();

        trace!("[+] response queued");
        Ok(())
    }
}
