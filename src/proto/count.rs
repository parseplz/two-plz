use crate::proto::config::ConnectionConfig;

#[derive(Debug)]
pub(super) struct Counts {
    /// Maximum number of locally initiated streams
    max_send_streams: usize,

    /// Current number of locally initiated streams
    num_send_streams: usize,

    /// Maximum number of remote initiated streams
    max_recv_streams: usize,

    /// Current number of remote initiated streams
    num_recv_streams: usize,

    /// Maximum number of pending locally reset streams
    max_local_reset_streams: usize,

    /// Current number of pending locally reset streams
    num_local_reset_streams: usize,

    /// Maximum number of locally reset streams due to protocol error across
    /// the lifetime of the connection.
    ///
    /// When this gets exceeded, we issue GOAWAYs.
    max_local_error_reset_streams: Option<usize>,

    /// Total number of locally reset streams due to protocol error across the
    /// lifetime of the connection.
    num_local_error_reset_streams: usize,
}

impl Counts {
    /// Create a new `Counts` using the provided configuration values.
    pub fn new(config: &ConnectionConfig) -> Self {
        Counts {
            max_send_streams: config
                .local_settings
                .max_concurrent_streams()
                .map(|v| v as usize)
                .unwrap_or(usize::MAX),
            num_send_streams: 0,
            max_recv_streams: config
                .peer_settings
                .max_concurrent_streams()
                .map(|v| v as usize)
                .unwrap_or(usize::MAX),
            num_recv_streams: 0,
            max_local_reset_streams: config.reset_stream_max,
            num_local_reset_streams: 0,
            max_local_error_reset_streams: config
                .local_max_error_reset_streams,
            num_local_error_reset_streams: 0,
        }
    }
}
