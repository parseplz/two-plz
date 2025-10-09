use crate::{
    frame::{DEFAULT_INITIAL_WINDOW_SIZE, StreamId},
    proto::{WindowSize, flow_control::FlowControl, store::Queue, stream},
};

struct Send {
    /// Queue of streams waiting for socket capacity to send a frame.
    pending_send: Queue<stream::NextSend>,

    /// Queue of streams waiting for window capacity to produce data.
    pending_capacity: Queue<stream::NextSendCapacity>,

    /// Queue of streams waiting for capacity due to max concurrency
    pending_open: Queue<stream::NextOpen>,

    /// Queue of streams waiting to be reset
    pending_reset: Queue<stream::NextResetExpire>,

    /// connection level
    flow: FlowControl,

    /// Stream ID of the last stream opened.
    last_opened_id: StreamId,

    /// Initial window size of locally initiated streams
    init_window_sz: WindowSize,
    // TODO
    //is_push_enabled: bool,
    //
    //is_extended_connect_protocol_enabled: bool,
}

impl Send {
    pub fn new() -> Send {
        Send {
            pending_send: Queue::new(),
            pending_capacity: Queue::new(),
            pending_open: Queue::new(),
            pending_reset: Queue::new(),
            flow: FlowControl::new(DEFAULT_INITIAL_WINDOW_SIZE),
            last_opened_id: StreamId::ZERO,
            init_window_sz: DEFAULT_INITIAL_WINDOW_SIZE,
        }
    }
}
