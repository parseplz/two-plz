use crate::proto::count::Counts;
use crate::proto::error::Initiator;
use crate::proto::recv::Open;
use crate::proto::send::Send;
use crate::proto::settings::SettingsAction;
use crate::proto::store::Store;
use crate::proto::stream::Stream;
use crate::proto::streams::Streams;
use crate::proto::{ProtoError, store};
use crate::proto::{recv::Recv, settings::SettingsHandler};
use crate::role::Role;
use std::io::Error;
use store::Entry;

use bytes::{Bytes, BytesMut};
use futures::StreamExt;
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::mpsc::{self, error::SendError},
};

use crate::{Headers, Reason, Reset, Settings, WindowUpdate, frame, proto};
use crate::{
    codec::{Codec, UserError},
    frame::{Frame, Ping, StreamId},
    proto::{
        config::ConnectionConfig,
        ping_pong::{PingAction, PingHandler},
    },
};

pub struct Connection<T, B> {
    pub codec: Codec<T, BytesMut>,
    streams: Streams<B>,
    ping_handler: PingHandler,
    settings_handler: SettingsHandler,
    role: Role,
}

impl<T, B> Connection<T, B>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    pub fn new(
        role: Role,
        config: ConnectionConfig,
        stream: Codec<T, BytesMut>,
    ) -> Self {
        let send = Send::new(&config, &role);
        let recv = Recv::new(&config, &role);
        Connection {
            ping_handler: PingHandler::new(),
            settings_handler: SettingsHandler::new(
                config.local_settings.clone(),
            ),
            codec: stream,
            role: role.clone(),
            streams: Streams::new(role, config),
        }
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
    ) -> Result<SettingsAction, ProtoError> {
        self.settings_handler.recv(local)
    }

    pub fn apply_local_settings(
        &mut self,
        settings: Settings,
    ) -> Result<(), ProtoError> {
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
        self.streams
            .apply_local_settings(&settings)
    }

    pub fn apply_remote_settings(
        &mut self,
        settings: Settings,
    ) -> Result<(), ProtoError> {
        self.streams
            .apply_remote_settings(&settings)
    }

    pub fn take_remote_settings(&mut self) -> Settings {
        self.settings_handler
            .take_remote_settings()
    }

    // ==== Window Update =====
    pub fn recv_connection_window_update(
        &mut self,
        size: u32,
    ) -> Result<(), ProtoError> {
        self.streams
            .recv_connection_window_update(size)
    }

    pub fn recv_stream_window_update(
        &mut self,
        id: StreamId,
        size: u32,
    ) -> Result<(), ProtoError> {
        self.streams
            .recv_stream_window_update(id, size)
    }

    }

    // ===== Test =====
    #[cfg(feature = "test-util")]
    pub fn read_frame(&mut self) -> Result<Frame, proto::ProtoError> {
        self.codec.read_frame()
    }
}
