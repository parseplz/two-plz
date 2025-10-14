use crate::builder::Role;
use crate::proto::send::Send;
use crate::proto::settings::SettingsAction;
use crate::proto::{recv::Recv, settings::SettingsHandler};
use std::io::Error;

use bytes::{Bytes, BytesMut};
use futures::StreamExt;
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::mpsc::{self, error::SendError},
};

use crate::{Settings, proto};
use crate::{
    codec::{Codec, UserError},
    frame::{Frame, Ping, StreamId},
    proto::{
        config::ConnectionConfig,
        ping_pong::{PingAction, PingHandler},
    },
};

// E = sender = to user
// U = receiver = from user
pub struct Connection<T, E, U> {
    config: ConnectionConfig,
    ping_handler: PingHandler,
    settings_handler: SettingsHandler,
    pub handler: Handler<E, U>,
    pub stream: Codec<T, BytesMut>,
    role: Role,
    send: Send,
    recv: Recv,
}

impl<T, E, U> Connection<T, E, U>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    pub fn new(
        role: Role,
        config: ConnectionConfig,
        stream: Codec<T, BytesMut>,
        local_settings: Settings,
    ) -> (Self, Handler<U, E>) {
        let (handler, user_handle) = Handler::build();
        let conn = Connection {
            config,
            ping_handler: PingHandler::new(),
            settings_handler: SettingsHandler::new(local_settings),
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
        self.ping_handler.handle(frame)
    }

    pub fn pending_pong(&mut self) -> Option<Ping> {
        self.ping_handler.pending_pong()
    }

    // ===== Settings =====
    pub fn handle_settings(
        &mut self,
        frame: Settings,
    ) -> Result<SettingsAction, proto::Error> {
        self.settings_handler.recv(frame)
    }

    #[cfg(feature = "test-util")]
    pub fn read_frame(&mut self) -> Result<Frame, proto::Error> {
        self.stream.read_frame()
    }

    pub fn apply_local_settings(&self, settings: Settings) {
        todo!()
    }
}

impl<T> ServerConnection<T>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    pub fn server(
        config: ConnectionConfig,
        stream: Codec<T, BytesMut>,
        local_settings: Settings,
    ) -> (
        Connection<T, ServerToUser, UserToServer>,
        Handler<UserToServer, ServerToUser>,
    ) {
        Connection::new(Role::Server, config, stream, local_settings)
    }
}

impl<T> ClientConnection<T>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    pub fn client(
        config: ConnectionConfig,
        stream: Codec<T, BytesMut>,
        local_settings: Settings,
    ) -> (
        Connection<T, ClientToUser, UserToClient>,
        Handler<UserToClient, ClientToUser>,
    ) {
        Connection::new(Role::Client, config, stream, local_settings)
    }
}

type ServerConnection<T> = Connection<T, ServerToUser, UserToServer>;
type ClientConnection<T> = Connection<T, ClientToUser, UserToClient>;
pub type ServerConnection<T> = Connection<T, ServerToUser, UserToServer>;
pub type ClientConnection<T> = Connection<T, ClientToUser, UserToClient>;

pub type ServerHandler = Handler<UserToServer, ServerToUser>;
pub type ClientHandler = Handler<UserToClient, ClientToUser>;

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

    pub async fn recv(&mut self) -> Option<E> {
        self.receiver.recv().await
    }

    async fn send(&mut self, item: T) -> Result<(), SendError<T>> {
        self.sender.send(item).await
    }
}
