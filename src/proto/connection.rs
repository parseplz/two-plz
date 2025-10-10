use futures::StreamExt;
use std::time::Duration;

use bytes::BytesMut;
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::mpsc,
};

use crate::{
    codec::Codec,
    frame::{Ping, Settings},
    preface::Role,
    proto::ping_pong::{PingPong, ReceivedPing},
};

pub struct Connection<T, E, U> {
    role: Role,
    config: ConnectionConfig,
    pub stream: Codec<T, BytesMut>,
    pub handler: Handler<E, U>,
    ping_pong: PingPong,
}

impl<T, E, U> Connection<T, E, U> {
    fn new(
        role: Role,
        config: ConnectionConfig,
        stream: Codec<T, BytesMut>,
    ) -> Self {
        todo!()
    }

    pub fn handle_ping(&mut self, frame: Ping) -> ReceivedPing {
        self.ping_pong.handle(frame)
    }
}

struct ConnectionConfig {
    /// Time to keep locally reset streams around before reaping.
    reset_stream_duration: Duration,

    local_settings: Settings,

    peer_settings: Settings,

    /// Initial target window size for new connections.
    initial_target_connection_window_size: Option<u32>,

    /// Maximum number of locally reset streams to keep at a time.
    reset_stream_max: usize,

    /// Maximum number of locally reset streams due to protocol error across
    /// the lifetime of the connection.
    ///
    /// When this gets exceeded, we issue GOAWAYs.
    local_max_error_reset_streams: Option<usize>,

    /// Initial maximum number of locally initiated (send) streams.
    /// After receiving a SETTINGS frame from the remote peer,
    /// the connection will overwrite this value with the
    /// MAX_CONCURRENT_STREAMS specified in the frame.
    /// If no value is advertised by the remote peer in the initial SETTINGS
    /// frame, it will be set to usize::MAX.
    max_concurrent_streams: usize,
}

enum ServerToUser {} // Request
enum UserToServer {} // Response

enum ClientToUser {} // Response
enum UserToClient {} // Request

pub struct Handler<T, E> {
    sender: mpsc::Sender<T>,
    pub receiver: mpsc::Receiver<E>,
}

impl<T, E> Handler<T, E> {
    fn build() -> (Handler<T, E>, Handler<E, T>) {
        let (ftx, frx) = mpsc::channel(1);
        let (stx, srx) = mpsc::channel(1);
        (
            Handler {
                sender: ftx,
                receiver: srx,
            },
            Handler {
                sender: stx,
                receiver: frx,
            },
        )
    }
}
