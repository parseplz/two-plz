use std::time::Duration;

use crate::{
    DEFAULT_INITIAL_WINDOW_SIZE, StreamId,
    proto::{
        WindowSize, buffer::Buffer, config::ConnectionConfig,
        flow_control::FlowControl, store::Queue, stream::NextResetExpire,
    },
    role::Role,
    stream_id::StreamIdOverflow,
};

#[derive(Debug)]
pub(super) struct Recv {
    /// Holds frames that are waiting to be read
    buffer: Buffer<Event>,

    /// Connection level flow control governing received data
    flow: FlowControl,

    /// Initial window size of remote initiated streams
    init_stream_window_sz: WindowSize,

    /// The stream ID of the last processed stream
    last_processed_id: StreamId,

    /// Any streams with a higher ID are ignored.
    ///
    /// This starts as MAX, but is lowered when a GOAWAY is received.
    ///
    /// > After sending a GOAWAY frame, the sender can discard frames for
    /// > streams initiated by the receiver with identifiers higher than
    /// > the identified last stream.
    max_stream_id: StreamId,

    /// The lowest stream ID that is still idle
    next_stream_id: Result<StreamId, StreamIdOverflow>,

    /// Streams that have pending window updates
    /// pending_window_updates: Queue<NextWindowUpdate>,
    /// Locally reset streams that should be reaped when they expire
    pending_reset_expired: Queue<NextResetExpire>,

    /// How long locally reset streams should ignore received frames
    reset_duration: Duration,
    //
    // TODO
    ///// If push promises are allowed to be received.
    //is_push_enabled: bool,
    //
    ///// If extended connect protocol is enabled.
    //is_extended_connect_protocol_enabled: bool,
}

#[derive(Debug)]
pub(super) enum Event {
    Header,
    Body,
    Trailer,
}

impl Recv {
    pub fn new(config: &ConnectionConfig, role: &Role) -> Recv {
        Recv {
            buffer: Buffer::new(),
            flow: FlowControl::new(
                config
                    .initial_connection_window_size
                    .unwrap_or(DEFAULT_INITIAL_WINDOW_SIZE),
            ),
            init_stream_window_sz: config
                .local_settings
                .initial_window_size()
                .unwrap_or(DEFAULT_INITIAL_WINDOW_SIZE),
            last_processed_id: StreamId::ZERO,
            max_stream_id: StreamId::MAX,
            next_stream_id: Ok(role.init_stream_id()),
            pending_reset_expired: Queue::new(),
            reset_duration: config.reset_stream_duration,
        }
    }
}
