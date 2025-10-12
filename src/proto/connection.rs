use futures::StreamExt;
use std::time::Duration;

use bytes::{Buf, BytesMut};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::mpsc,
};

use crate::{
    codec::{Codec, UserError},
    frame::{Frame, Ping, Settings, StreamId},
    preface::Role,
    proto::{
        config::{ConnectionConfig, PeerRole},
        ping_pong::{PingAction, PingPong},
    },
};

// E = to user
// U = from user
pub struct Connection<T, E, U> {
    config: ConnectionConfig,
    pub ping_pong: PingPong,
    pub handler: Handler<E, U>,
    pub stream: Codec<T, BytesMut>,
    role: PeerRole,
}

impl<T, E, U> Connection<T, E, U>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    pub fn new(
        role: PeerRole,
        config: ConnectionConfig,
        stream: Codec<T, BytesMut>,
    ) -> Self {
        todo!()
    }

    pub fn server(
        config: ConnectionConfig,
        stream: Codec<T, BytesMut>,
    ) -> Self {
        Connection::new(PeerRole::Server, config, stream)
    }

    pub fn client(
        config: ConnectionConfig,
        stream: Codec<T, BytesMut>,
    ) -> Self {
        Connection::new(
            PeerRole::Client {
                initial_max_send_streams: 1,
                stream_id: StreamId::from(1),
            },
            config,
            stream,
        )
    }

    // ===== Codec =====
    pub fn buffer(&mut self, item: Frame<BytesMut>) -> Result<(), UserError> {
        self.stream.buffer(item)
    }

    // ===== Ping =====
    pub fn handle_ping(&mut self, frame: Ping) -> PingAction {
        self.ping_pong.handle(frame)
    }
}

pub enum ServerToUser {} // Request
pub enum UserToServer {} // Response

pub enum ClientToUser {} // Response
pub enum UserToClient {} // Request

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
