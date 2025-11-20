use bytes::Bytes;

use crate::Frame;
use crate::Reason;
use crate::StreamId;
use crate::proto::streams::Counts;
use crate::proto::streams::Ptr;
use crate::proto::streams::buffer::Buffer;
use crate::{
    proto::{
        ProtoError,
        config::ConnectionConfig,
        streams::{Recv, Send},
    },
    role::Role,
};
use std::task::Waker;

#[derive(Debug)]
pub struct Actions {
    /// Manages state transitions initiated by receiving frames
    pub recv: Recv,

    /// Manages state transitions initiated by sending frames
    pub send: Send,

    /// Task that calls `poll_complete`.
    pub task: Option<Waker>,

    /// If the connection errors, a copy is kept for any StreamRefs.
    pub conn_error: Option<ProtoError>,
}

impl Actions {
    pub fn new(role: Role, config: ConnectionConfig) -> Self {
        Actions {
            recv: Recv::new(&config, &role),
            send: Send::new(&config, &role),
            task: None,
            conn_error: None,
        }
    }

    pub fn reset_on_recv_stream_err(
        &mut self,
        buffer: &mut Buffer<Frame<Bytes>>,
        stream: &mut Ptr,
        counts: &mut Counts,
        res: Result<(), ProtoError>,
    ) -> Result<(), ProtoError> {
        if let Err(ProtoError::Reset(stream_id, reason, initiator)) = res {
            debug_assert_eq!(stream_id, stream.id);

            if counts.can_inc_num_local_error_resets() {
                counts.inc_num_local_error_resets();
                // Reset the stream.
                self.send.send_reset(
                    reason,
                    initiator,
                    stream,
                    buffer,
                    counts,
                    &mut self.task,
                );
                self.recv
                    .enqueue_reset_expiration(stream, counts);
                stream.notify_recv();
                Ok(())
            } else {
                Err(ProtoError::library_go_away_data(
                    Reason::ENHANCE_YOUR_CALM,
                    "too_many_internal_resets",
                ))
            }
        } else {
            res
        }
    }

    /// Check whether the stream was present in the past
    pub fn ensure_not_idle(
        &mut self,
        role: Role,
        id: StreamId,
    ) -> Result<(), Reason> {
        let next_id = if role.is_local_init(id) {
            self.send.next_stream_id
        } else {
            self.recv.next_stream_id
        };

        if let Ok(next) = next_id
            && id >= next
        {
            return Err(Reason::PROTOCOL_ERROR);
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
    pub fn is_forgotten_stream(&self, role: &Role, id: StreamId) -> bool {
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
