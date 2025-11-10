use std::time::Duration;

use crate::frame::Settings;

#[derive(Clone, Debug)]
pub struct ConnectionConfig {
    /// Initial target window size for new connections.
    pub initial_connection_window_size: Option<u32>,

    /// Maximum number of locally reset streams due to protocol error across
    /// the lifetime of the connection.
    ///
    /// When this gets exceeded, we issue GOAWAYs.
    pub local_max_error_reset_streams: Option<usize>,

    /// Maximum number of locally reset streams to keep at a time.
    pub local_reset_stream_max: usize,

    /// Maximum number of remote reset streams to keep at a time.
    pub remote_reset_stream_max: usize,

    /// Time to keep locally reset streams around before reaping.
    pub reset_stream_duration: Duration,

    /// settings
    pub local_settings: Settings,
    pub peer_settings: Settings,
}
