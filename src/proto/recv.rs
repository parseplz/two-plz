use std::time::Duration;

use crate::{
    StreamId,
    proto::{
        WindowSize, buffer::Buffer, flow_control::FlowControl, store::Queue,
        stream::NextResetExpire,
    },
    stream_id::StreamIdOverflow,
};

#[derive(Debug)]
pub(super) struct Recv {
    /// Initial window size of remote initiated streams
    init_window_sz: WindowSize,

    /// Connection level flow control governing received data
    flow: FlowControl,

    /// Amount of connection window capacity currently used by outstanding streams.
    in_flight_data: WindowSize,

    /// The lowest stream ID that is still idle
    next_stream_id: Result<StreamId, StreamIdOverflow>,

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

    ///// Streams that have pending window updates
    //pending_window_updates: Queue<NextWindowUpdate>,
    /// Locally reset streams that should be reaped when they expire
    pending_reset_expired: Queue<NextResetExpire>,

    /// How long locally reset streams should ignore received frames
    reset_duration: Duration,

    /// Holds frames that are waiting to be read
    buffer: Buffer<Event>,

    /// If push promises are allowed to be received.
    is_push_enabled: bool,

    /// If extended connect protocol is enabled.
    is_extended_connect_protocol_enabled: bool,
}

#[derive(Debug)]
pub(super) enum Event {
    Header,
    Body,
    Trailer,
}

impl Recv {
    pub fn new() -> Recv {
        todo!()
    }
}
