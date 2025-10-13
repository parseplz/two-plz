use crate::proto::recv::Recv;
use crate::proto::send::Send;
use std::io::Error;

use bytes::BytesMut;
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::mpsc::{self, error::SendError},
};

#[cfg(feature = "test-util")]
use crate::proto;
use crate::{
    codec::{Codec, UserError},
    frame::{Frame, Ping, StreamId},
    proto::{
        config::{ConnectionConfig, PeerRole},
        ping_pong::{PingAction, PingPong},
    },
};

// E = sender = to user
// U = receiver = from user
pub struct Connection<T, E, U> {
    config: ConnectionConfig,
    ping_pong: PingPong,
    pub handler: Handler<E, U>,
    pub stream: Codec<T, BytesMut>,
    role: PeerRole,
    send: Send,
    recv: Recv,
}

impl<T, E, U> Connection<T, E, U>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    pub fn new(
        role: PeerRole,
        config: ConnectionConfig,
        stream: Codec<T, BytesMut>,
    ) -> (Self, Handler<U, E>) {
        let (handler, user_handle) = Handler::build();
        let conn = Connection {
            config,
            ping_pong: PingPong::new(),
            handler,
            stream,
            role,
            send: Send::new(),
            recv: Recv::new(),
        };
        (conn, user_handle)
    }

    // ===== Codec =====
    pub fn buffer(&mut self, item: Frame<BytesMut>) -> Result<(), UserError> {
        self.stream.buffer(item)
    }

    // ===== Ping =====
    pub fn handle_ping(&mut self, frame: Ping) -> PingAction {
        self.ping_pong.handle(frame)
    }

    pub fn pending_pong(&mut self) -> Option<Ping> {
        self.ping_pong.pending_pong()
    }

    #[cfg(feature = "test-util")]
    pub fn read_frame(&mut self) -> Result<Frame, proto::Error> {
        self.stream.read_frame()
    }
}

impl<T> ServerConnection<T>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    pub fn server(
        config: ConnectionConfig,
        stream: Codec<T, BytesMut>,
    ) -> (
        Connection<T, ServerToUser, UserToServer>,
        Handler<UserToServer, ServerToUser>,
    ) {
        Connection::new(PeerRole::Server, config, stream)
    }
}

impl<T> ClientConnection<T>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    pub fn client(
        config: ConnectionConfig,
        stream: Codec<T, BytesMut>,
    ) -> (
        Connection<T, ClientToUser, UserToClient>,
        Handler<UserToClient, ClientToUser>,
    ) {
        Connection::new(
            PeerRole::Client {
                initial_max_send_streams: 1,
                stream_id: StreamId::from(1),
            },
            config,
            stream,
        )
    }
}

type ServerConnection<T> = Connection<T, ServerToUser, UserToServer>;
type ClientConnection<T> = Connection<T, ClientToUser, UserToClient>;

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

    async fn recv(&mut self) -> Option<E> {
        self.receiver.recv().await
    }

    async fn send(&mut self, item: T) -> Result<(), SendError<T>> {
        self.sender.send(item).await
    }
}
