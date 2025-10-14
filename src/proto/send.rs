use crate::{
    builder::Role,
    frame::{DEFAULT_INITIAL_WINDOW_SIZE, Frame, StreamId},
    proto::{
        WindowSize,
        buffer::Buffer,
        config::ConnectionConfig,
        flow_control::FlowControl,
        store::{Ptr, Queue},
        stream,
    },
    stream_id::StreamIdOverflow,
};

pub struct Send {
    /// Initial window size of locally initiated streams
    init_window_sz: WindowSize,

    /// connection level
    flow: FlowControl,

    /// Stream ID of the last stream opened.
    last_opened_id: StreamId,

    /// Any streams with a higher ID are ignored.
    ///
    /// This starts as MAX, but is lowered when a GOAWAY is received.
    ///
    /// > After sending a GOAWAY frame, the sender can discard frames for
    /// > streams initiated by the receiver with identifiers higher than
    /// > the identified last stream.
    max_stream_id: StreamId,

    // TODO: make this configurable
    // hyper Builder::StreamId
    /// Stream identifier to use for next initialized stream.
    next_stream_id: Result<StreamId, StreamIdOverflow>,

    /// Queue of streams waiting for socket capacity to send a frame.
    pending_send: Queue<stream::NextSend>,

    /// Queue of streams waiting for window capacity to produce data.
    pending_capacity: Queue<stream::NextSendCapacity>,

    /// Queue of streams waiting for capacity due to max concurrency
    pending_open: Queue<stream::NextOpen>,

    /// Queue of streams waiting to be reset
    pending_reset: Queue<stream::NextResetExpire>,
    // TODO
    //is_push_enabled: bool,
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

    /// Queue a frame to be sent to the remote
    pub fn queue_frame<B>(
        &mut self,
        frame: Frame<B>,
        buffer: &mut Buffer<Frame<B>>,
        stream: &mut Ptr,
    ) {
        // Queue the frame in the buffer
        stream
            .pending_send
            .push_back(buffer, frame);
        self.schedule_send(stream);
    }

    pub fn schedule_send(&mut self, stream: &mut Ptr) {
        // If the stream is waiting to be opened, nothing more to do.
        if stream.is_send_ready() {
            tracing::trace!(?stream.id, "schedule_send");
            // Queue the stream
            self.pending_send.push(stream);
        }
    }
}
