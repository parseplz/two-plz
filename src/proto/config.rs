use std::time::Duration;

use crate::frame::{Settings, StreamId};

#[derive(Clone, Debug)]
pub struct ConnectionConfig {
    /// Initial target window size for new connections.
    pub initial_target_connection_window_size: Option<u32>,

    /// Maximum number of locally reset streams due to protocol error across
    /// the lifetime of the connection.
    ///
    /// When this gets exceeded, we issue GOAWAYs.
    pub local_max_error_reset_streams: Option<usize>,
    /// Time to keep locally reset streams around before reaping.
    pub reset_stream_duration: Duration,
    /// Maximum number of locally reset streams to keep at a time.
    pub reset_stream_max: usize,
    pub role: PeerRole,

    /// TODO: may be store in send and recv settings ?
    pub settings: Settings,
    pub peer_settings: Settings,
}

#[derive(Clone, Debug)]
pub enum PeerRole {
    Server,
    Client {
        initial_max_send_streams: usize,
        stream_id: StreamId,
    },
}
