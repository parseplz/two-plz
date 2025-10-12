use bytes::BytesMut;
use std::time::Instant;

use crate::{
    frame::StreamId,
    proto::{
        WindowSize,
        buffer::Deque,
        flow_control::FlowControl,
        store::{self, Key, Next},
    },
};

// ===== Queues =====
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

    // ===== Send =====
    pub send_flow: FlowControl,
    body: Option<BytesMut>,

    /// Next Send
    pub next_pending_send: Option<Key>,
    pub is_pending_send: bool,
    pub pending_send: Deque, // frames

    /// Next capacity.
    pub next_pending_send_capacity: Option<Key>,
    pub is_pending_send_capacity: bool,

    /// Next Open
    pub next_open: Option<Key>,
    pub is_pending_open: bool,

    // ===== Recv =====
    pub recv_flow: FlowControl,
    pub pending_recv: Deque, // Events
    pub content_length: ContentLength,

    // ===== Reset =====
    /// The time when this stream may have been locally reset.
    pub reset_at: Option<Instant>,
    pub next_reset_expire: Option<Key>,
    // TODO
    //state: State,
    //pub content_length: ContentLength,
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
            // === send ===
            send_flow,
            body: None,
            // next send
            next_pending_send: None,
            is_pending_send: false,
            pending_send: Deque::new(),
            // next capacity
            next_pending_send_capacity: None,
            is_pending_send_capacity: false,
            // next open
            next_open: None,
            is_pending_open: false,
            // === recv ===
            recv_flow,
            pending_recv: Deque::new(),
            content_length: ContentLength::Omitted,
            // === reset ===
            reset_at: None,
            next_reset_expire: None,
        }
    }

    fn is_closed(&self) -> bool {
        // TODO
        //self.state.is_closed() &&
        self.pending_send.is_empty()
    }

    pub fn is_send_ready(&self) -> bool {
        !self.is_pending_open
    }
}

// ===== Queue =====
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

impl store::Next for NextSendCapacity {
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

impl store::Next for NextOpen {
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

impl store::Next for NextResetExpire {
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
