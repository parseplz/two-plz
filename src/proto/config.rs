use std::time::Duration;

use tracing::error;

use crate::{
    builder::Builder,
    frame::{Settings, StreamId},
    proto::DEFAULT_LOCAL_RESET_COUNT_MAX,
};

#[derive(Clone, Debug)]
pub struct ConnectionConfig {
    /// Initial target window size for new connections.
    pub initial_connection_window_size: Option<u32>,
    /// Maximum number of locally reset streams due to protocol error across
    /// the lifetime of the connection.
    ///
    /// When this gets exceeded, we issue GOAWAYs.
    pub local_max_error_reset_streams: Option<usize>,
    /// Time to keep locally reset streams around before reaping.
    pub reset_stream_duration: Duration,
    /// Maximum number of locally reset streams to keep at a time.
    pub reset_stream_max: usize,

    /// settings
    pub local_settings: Settings,
    pub peer_settings: Settings,
}

impl From<(Builder, Settings)> for ConnectionConfig {
    fn from((builder, peer_settings): (Builder, Settings)) -> Self {
        Self {
            initial_connection_window_size: builder
                .initial_connection_window_size,
            local_max_error_reset_streams: builder
                .local_max_error_reset_streams,
            reset_stream_duration: builder.reset_stream_duration,
            reset_stream_max: builder.reset_stream_max,
            local_settings: builder.settings,
            peer_settings,
        }
    }
}
