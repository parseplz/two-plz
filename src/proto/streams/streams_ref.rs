use http_plz::{Request, Response};
use std::sync::{Arc, Mutex};
use tracing::Level;

use bytes::Bytes;
use tracing::{span, trace};

use crate::{
    codec::UserError,
    error::Reason,
    frame::StreamId,
    message::TwoTwoFrame,
    proto::{
        error::Initiator,
        streams::{
            inner::Inner,
            opaque_streams_ref::OpaqueStreamRef,
            send_buffer::SendBuffer,
            store::{Ptr, Resolve},
            streams::queue_body_trailer,
        },
    },
};

/// Reference to the stream state
#[derive(Debug)]
pub(crate) struct StreamRef {
    pub opaque: OpaqueStreamRef,
    pub send_buffer: Arc<SendBuffer<Bytes>>,
}

impl StreamRef {
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
        response: Response,
    ) -> Result<(), UserError> {
        let mut me = self.opaque.inner.lock().unwrap();
        let me = &mut *me;
        let stream = me.store.resolve(self.opaque.key);
        let actions = &mut me.actions;
        let span = span!(Level::TRACE, "send response|", "{:?}| ", stream.id);
        let _enter = span.enter();

        if stream.state.is_remote_reset() {
            trace!("remote reset");
            if let Some(task) = actions.task.take() {
                task.wake()
            }
            return Err(UserError::InactiveStreamId);
        }

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
            })?;
        trace!("added| header");

        let mut stream = me.store.resolve(self.opaque.key);

        queue_body_trailer(
            &mut stream,
            data_frame,
            trailer_frame,
            send_buffer,
        );

        trace!("response queued");
        Ok(())
    }

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

    pub fn stream_id(&self) -> StreamId {
        self.opaque.stream_id()
    }
}

impl Clone for StreamRef {
    fn clone(&self) -> Self {
        StreamRef {
            opaque: self.opaque.clone(),
            send_buffer: self.send_buffer.clone(),
        }
    }
}
