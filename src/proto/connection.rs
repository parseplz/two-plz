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
        self.count
            .apply_remote_settings(&settings);
        self.send
            .apply_remote_settings(&settings, &mut self.store)
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
        self.send
            .recv_connection_window_update(size)
            .map_err(ProtoError::library_go_away)
    }

    pub fn recv_stream_window_update(
        &mut self,
        id: StreamId,
        size: u32,
    ) -> Result<(), ProtoError> {
        if let Some(mut stream) = self.store.find_mut(&id) {
            if let Err(e) = self
                .send
                .recv_stream_window_update(&mut stream, size)
            {
                // send reset
                self.send.send_reset(
                    Reason::FLOW_CONTROL_ERROR,
                    Initiator::Library,
                    &mut stream,
                )
            }
            Ok(())
        } else {
            self.ensure_not_idle(id)
                .map_err(ProtoError::library_go_away)
        }
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

    /// Check if we possibly could have processed and since forgotten this stream.
    ///
    /// If we send a RST_STREAM for a stream, we will eventually "forget" about
    /// the stream to free up memory. It's possible that the remote peer had
    /// frames in-flight, and by the time we receive them, our own state is
    /// gone. We *could* tear everything down by sending a GOAWAY, but it
    /// is more likely to be latency/memory constraints that caused this,
    /// and not a bad actor. So be less catastrophic, the spec allows
    /// us to send another RST_STREAM of STREAM_CLOSED.
    fn may_have_forgotten_stream(&self, id: StreamId) -> bool {
        if id.is_zero() {
            return false;
        }

        let next = if self.role.is_local_init(id) {
            self.send.next_stream_id
        } else {
            self.recv.next_stream_id
        };

        if let Ok(next_id) = next {
            debug_assert_eq!(
                id.is_server_initiated(),
                next_id.is_server_initiated(),
            );
            id < next_id
        } else {
            true
        }
    }

    // ===== Test =====
    #[cfg(feature = "test-util")]
    pub fn read_frame(&mut self) -> Result<Frame, proto::ProtoError> {
        self.codec.read_frame()
    }
}
