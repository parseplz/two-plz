use std::time::Duration;

use tracing::error;

use crate::frame::{Settings, StreamId};

#[derive(Clone, Debug)]
pub enum PeerRole {
    Server,
    Client {
        /// Initial maximum number of locally initiated (send) streams.
        /// After receiving a SETTINGS frame from the remote peer,
        /// the connection will overwrite this value with the
        /// MAX_CONCURRENT_STREAMS specified in the frame.
        /// If no value is advertised by the remote peer in the initial SETTINGS
        /// frame, it will be set to usize::MAX.
        initial_max_send_streams: usize,
        stream_id: StreamId,
    },
}

impl PeerRole {
    fn server() -> PeerRole {
        PeerRole::Server
    }

    fn client() -> PeerRole {
        PeerRole::Client {
            initial_max_send_streams: usize::MAX,
            stream_id: StreamId::ZERO,
        }
    }

    fn set_max_send_streams(&mut self, max_send_streams: usize) {
        if let PeerRole::Client {
            initial_max_send_streams,
            ..
        } = self
        {
            *initial_max_send_streams = max_send_streams;
        } else {
            error!("set_max_send_streams called on server")
        }
    }

    fn set_initial_stream_id(&mut self, id: StreamId) {
        if let PeerRole::Client {
            stream_id,
            ..
        } = self
        {
            *stream_id = id;
        } else {
            error!("set_initial_stream_id called on server")
        }
    }
}

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
