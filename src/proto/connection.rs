use crate::proto::count::Counts;
use crate::proto::send::Send;
use crate::proto::settings::SettingsAction;
use crate::proto::store::Store;
use crate::proto::{recv::Recv, settings::SettingsHandler};
use crate::role::Role;
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
    count: Counts,
    pub handler: Handler<E, U>,
    ping_handler: PingHandler,
    recv: Recv,
    role: Role,
    send: Send,
    store: Store,
    settings_handler: SettingsHandler,
    pub codec: Codec<T, BytesMut>,
}

impl<T, E, U> Connection<T, E, U>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    pub fn new(
        role: Role,
        config: ConnectionConfig,
        stream: Codec<T, BytesMut>,
    ) -> (Self, Handler<U, E>) {
        let (handler, user_handle) = Handler::build();
        let send = Send::new(&config, &role);
        let recv = Recv::new(&config, &role);
        let conn = Connection {
            ping_handler: PingHandler::new(),
            settings_handler: SettingsHandler::new(
                config.local_settings.clone(),
            ),
            handler,
            codec: stream,
            role,
            count: Counts::new(&config),
            send,
            recv,
            store: Store::new(),
        };
        (conn, user_handle)
    }

    // ===== Codec =====
    pub fn buffer(&mut self, item: Frame<BytesMut>) -> Result<(), UserError> {
        self.codec.buffer(item)
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
        local: Settings,
    ) -> Result<SettingsAction, proto::ProtoError> {
        self.settings_handler.recv(local)
    }

    pub fn apply_local_settings(
        &mut self,
        settings: Settings,
    ) -> Result<(), proto::ProtoError> {
        if let Some(max) = settings.max_frame_size() {
            self.codec
                .set_max_recv_frame_size(max as usize);
        }

        if let Some(max) = settings.max_header_list_size() {
            self.codec
                .set_max_recv_header_list_size(max as usize);
        }

        if let Some(val) = settings.header_table_size() {
            self.codec
                .set_recv_header_table_size(val as usize);
        }
        self.recv
            .apply_local_settings(&settings, &mut self.store)
    }

    pub fn apply_remote_settings(
        &mut self,
        settings: Settings,
    ) -> Result<(), proto::ProtoError> {
        self.count
            .apply_remote_settings(&settings);
        self.send
            .apply_remote_settings(&settings, &mut self.store)
    }

    pub fn take_remote_settings(&mut self) -> Settings {
        self.settings_handler
            .take_remote_settings()
    }


    // ===== Misc =====
    /// Check whether the stream was present in the past
    fn ensure_not_idle(&mut self, id: StreamId) -> Result<(), Reason> {
        let next_id = if self.role.is_local_init(id) {
            self.send.next_stream_id
        } else {
            self.recv.next_stream_id
        };

        if let Ok(next) = next_id {
            if id >= next {
                return Err(Reason::PROTOCOL_ERROR);
            }
        }
        Ok(())
    }

    // ===== Test =====
    #[cfg(feature = "test-util")]
    pub fn read_frame(&mut self) -> Result<Frame, proto::ProtoError> {
        self.codec.read_frame()
    }
}

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
