use crate::proto::streams::buffer::Deque;
use crate::proto::streams::flow_control::FlowControl;
use crate::proto::streams::store::{self, Key, Next, Queue};
use crate::{Reason, proto::streams::state::State};
use crate::proto::error::Initiator;
use bytes::BytesMut;
use std::time::Instant;

use crate::{
    frame::StreamId,
    proto::{
        WindowSize,
    },
};

// ===== Queues =====
// recv
#[derive(Debug)]
pub(super) struct NextAccept;

// send
#[derive(Debug)]
pub(super) struct NextSend;

#[derive(Debug)]
pub(super) struct NextSendCapacity;

#[derive(Debug)]
pub(super) struct NextOpen;

#[derive(Debug)]
pub(super) struct NextResetExpire;

#[derive(Debug)]
pub struct Stream {
    pub(crate) id: StreamId,
    pub state: State,

    /// Number of outstanding handles pointing to this stream
    pub ref_count: usize,

    /// Set to `true` when the stream is counted against the connection's max
    /// concurrent streams.
    pub is_counted: bool,

    // ===== Send =====
    pub send_flow: FlowControl,

    /// Next Send
    pub next_pending_send: Option<Key>,
    pub is_pending_send: bool,
    pub pending_send: Deque, // frames

    /// Next capacity.
    pub next_pending_send_capacity: Option<Key>,
    pub is_pending_send_capacity: bool,
    /// Set to true when the send capacity has been incremented
    pub send_capacity_inc: bool,

    /// Next Open
    pub next_open: Option<Key>,
    pub is_pending_open: bool,

    // ===== Recv =====
    pub recv_flow: FlowControl,
    pub content_length: ContentLength,
    /// When the RecvStream drop occurs, no data should be received.
    pub is_recv: bool,

    /// Next Complete

    /// Next Accept
    pub next_pending_accept: Option<Key>,
    pub is_pending_accept: bool,
    pub pending_recv: Deque, // Events

    // ===== Reset =====
    /// The time when this stream may have been locally reset.
    pub reset_at: Option<Instant>,
    pub next_reset_expire: Option<Key>,

    /// ===== Push Promise =====
    /// The stream's pending push promises
    pub pending_push_promises: Queue<NextAccept>,


}

impl Stream {
    pub fn new(
        id: StreamId,
        init_send_window: WindowSize,
        init_recv_window: WindowSize,
    ) -> Stream {
        let send_flow = FlowControl::new(init_send_window);
        let recv_flow = FlowControl::new(init_recv_window);

        Stream {
            id,
            state: State::default(),
            ref_count: 0,
            is_counted: false,
            // === send ===
            send_flow,
            // next send
            next_pending_send: None,
            is_pending_send: false,
            pending_send: Deque::new(),
            // next capacity
            next_pending_send_capacity: None,
            is_pending_send_capacity: false,
            send_capacity_inc: false,
            // next open
            next_open: None,
            is_pending_open: false,
            // === recv ===
            recv_flow,
            content_length: ContentLength::Omitted,
            is_recv: true,
            // next accept
            next_pending_accept: None,
            is_pending_accept: false,
            pending_recv: Deque::new(),
            // === reset ===
            reset_at: None,
            next_reset_expire: None,
            // push promise
            pending_push_promises: Queue::new(),
        }
    }

    pub fn is_closed(&self) -> bool {
        self.state.is_closed() && self.pending_send.is_empty()
    }

    /// Returns true if stream is currently being held for some time because of
    /// a local reset.
    pub fn is_pending_reset_expiration(&self) -> bool {
        self.reset_at.is_some()
    }

    /// Returns true if the stream is no longer in use
    pub fn is_released(&self) -> bool {
        // The stream is closed and fully flushed
        self.is_closed() &&
            // There are no more outstanding references to the stream
            self.ref_count == 0 &&
            // The stream is not in any queue
            !self.is_pending_send && !self.is_pending_send_capacity &&
            !self.is_pending_accept && 
            !self.is_pending_open && self.reset_at.is_none()
    }

    pub fn is_send_ready(&self) -> bool {
        !self.is_pending_open
    }

    pub(super) fn set_reset(&mut self, reason: Reason, initiator: Initiator) {
        self.state
            .set_reset(self.id, reason, initiator);
    }


    pub fn ensure_content_length_zero(&self) -> Result<(), ()> {
        match self.content_length {
            ContentLength::Remaining(0) => Ok(()),
            ContentLength::Remaining(_) => Err(()),
            _ => Ok(()),
        }
    }
}

// ===== Queue =====
// recv
impl Next for NextAccept {
    fn next(stream: &Stream) -> Option<store::Key> {
        stream.next_pending_accept
    }

    fn set_next(stream: &mut Stream, key: Option<store::Key>) {
        stream.next_pending_accept = key;
    }

    fn take_next(stream: &mut Stream) -> Option<store::Key> {
        stream.next_pending_accept.take()
    }

    fn is_queued(stream: &Stream) -> bool {
        stream.is_pending_accept
    }

    fn set_queued(stream: &mut Stream, val: bool) {
        stream.is_pending_accept = val;
    }
}

// send
impl Next for NextSend {
    fn next(stream: &Stream) -> Option<store::Key> {
        stream.next_pending_send
    }

    fn set_next(stream: &mut Stream, key: Option<store::Key>) {
        stream.next_pending_send = key;
    }

    fn take_next(stream: &mut Stream) -> Option<store::Key> {
        stream.next_pending_send.take()
    }

    fn is_queued(stream: &Stream) -> bool {
        stream.is_pending_send
    }

    fn set_queued(stream: &mut Stream, val: bool) {
        if val {
            // ensure that stream is not queued for being opened
            // if it's being put into queue for sending data
            debug_assert!(!stream.is_pending_open);
        }
        stream.is_pending_send = val;
    }
}

impl Next for NextSendCapacity {
    fn next(stream: &Stream) -> Option<store::Key> {
        stream.next_pending_send_capacity
    }

    fn set_next(stream: &mut Stream, key: Option<store::Key>) {
        stream.next_pending_send_capacity = key;
    }

    fn take_next(stream: &mut Stream) -> Option<store::Key> {
        stream.next_pending_send_capacity.take()
    }

    fn is_queued(stream: &Stream) -> bool {
        stream.is_pending_send_capacity
    }

    fn set_queued(stream: &mut Stream, val: bool) {
        stream.is_pending_send_capacity = val;
    }
}

impl Next for NextOpen {
    fn next(stream: &Stream) -> Option<store::Key> {
        stream.next_open
    }

    fn set_next(stream: &mut Stream, key: Option<store::Key>) {
        stream.next_open = key;
    }

    fn take_next(stream: &mut Stream) -> Option<store::Key> {
        stream.next_open.take()
    }

    fn is_queued(stream: &Stream) -> bool {
        stream.is_pending_open
    }

    fn set_queued(stream: &mut Stream, val: bool) {
        if val {
            // ensure that stream is not queued for being sent
            // if it's being put into queue for opening the stream
            debug_assert!(!stream.is_pending_send);
        }
        stream.is_pending_open = val;
    }
}

impl Next for NextResetExpire {
    fn next(stream: &Stream) -> Option<store::Key> {
        stream.next_reset_expire
    }

    fn set_next(stream: &mut Stream, key: Option<store::Key>) {
        stream.next_reset_expire = key;
    }

    fn take_next(stream: &mut Stream) -> Option<store::Key> {
        stream.next_reset_expire.take()
    }

    fn is_queued(stream: &Stream) -> bool {
        stream.reset_at.is_some()
    }

    fn set_queued(stream: &mut Stream, val: bool) {
        if val {
            stream.reset_at = Some(Instant::now());
        } else {
            stream.reset_at = None;
        }
    }
}

#[derive(Debug)]
pub enum ContentLength {
    Omitted,
    Head,
    Remaining(u64),
}

impl ContentLength {
    pub fn is_head(&self) -> bool {
        matches!(*self, Self::Head)
    }
}
