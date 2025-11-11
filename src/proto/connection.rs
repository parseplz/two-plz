use std::pin::Pin;
use std::task::Context;
use std::task::Poll;

use crate::Data;
use crate::Reason;
use crate::WindowUpdate;
use crate::proto::ProtoError;
use crate::proto::WindowSize;
use crate::proto::error::Initiator;
use crate::proto::settings::SettingsAction;
use crate::proto::settings::SettingsHandler;
use crate::proto::streams::streams::Streams;
use crate::proto::streams::streams_ref::StreamRef;
use crate::role::Role;

use bytes::BytesMut;
use futures::Stream;
use tokio::io::{AsyncRead, AsyncWrite};
use tracing::debug;
use tracing::trace;

use crate::{Headers, Reset, Settings};
use crate::{
    codec::{Codec, UserError},
    frame::{Frame, Ping, StreamId},
    proto::{
        config::ConnectionConfig,
        ping_pong::{PingAction, PingHandler},
    },
};

pub enum ReadAction {
    Continue,
    NeedsFlush,
}

#[derive(Debug)]
enum ConnectionState {
    /// Currently open in a sane state
    Open,

    /// The codec must be flushed
    Closing(Reason, Initiator),

    /// In a closed state
    Closed(Reason, Initiator),
}

pub struct Connection<T, B> {
    pub codec: Codec<T, BytesMut>,
    streams: Streams<B>,
    ping_handler: PingHandler,
    settings_handler: SettingsHandler,
    role: Role,
    state: ConnectionState,
    span: tracing::Span,
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
        Connection {
            state: ConnectionState::Open,
            ping_handler: PingHandler::new(),
            settings_handler: SettingsHandler::new(
                config.local_settings.clone(),
            ),
            codec: stream,
            role: role.clone(),
            span: tracing::debug_span!("connection| "),
            streams: Streams::new(role, config),
        }
    }

    // ===== Server =====
    pub fn next_accept(&mut self) -> Option<StreamRef<B>> {
        self.streams.next_accept()
    }

    // ===== Codec =====
    pub fn buffer(&mut self, item: Frame<BytesMut>) -> Result<(), UserError> {
        self.codec.buffer(item)
    }

    // ======== FRAMES ============
    pub fn recv_frame(
        &mut self,
        frame: Frame,
    ) -> Result<ReadAction, ProtoError> {
        match frame {
            Frame::Data(data) => return self.recv_data(data),
            Frame::Headers(headers) => self.streams.recv_header(headers),
            Frame::Priority(priority) => todo!(),
            Frame::Reset(reset) => self.streams.recv_reset(reset),
            Frame::Settings(settings) => {
                let _ = self.handle_settings(settings);
                Ok(())
            }
            Frame::PushPromise(push_promise) => todo!(),
            Frame::Ping(ping) => {
                let action = self.ping_handler.handle(ping);
                // TODO
                //if action.is_shutdown() {
                //    todo!()
                //}
                todo!()
            }
            Frame::GoAway(go_away) => todo!(),
            Frame::WindowUpdate(window_update) => {
                let id = window_update.stream_id();
                let inc = window_update.size_increment();
                if id.is_zero() {
                    self.recv_connection_window_update(inc)
                } else {
                    self.recv_stream_window_update(id, inc)
                }
            }
        };

    // ===== Data =====
    pub fn recv_data(&mut self, data: Data) -> Result<ReadAction, ProtoError> {
        let stream_id = data.stream_id();
        self.streams.recv_data(data)?;
        let mut need_flush = false;
        // check connection and stream window_update
        // size of window update = 13 bytes
        // max frame size = 16kb
        // so we can buffer multiple frames
        if let Some(size) = self
            .streams
            .should_send_connection_window_update()
        {
            need_flush = true;
            let frame = WindowUpdate::new(StreamId::ZERO, size);
            // safe to unwrap
            self.buffer(frame.into()).unwrap();
        }
        if let Some(size) = self
            .streams
            .should_send_stream_window_update(stream_id)
        {
            need_flush = true;
            let frame = WindowUpdate::new(stream_id, size);
            // safe to unwrap
            self.buffer(frame.into()).unwrap();
        }
        if need_flush {
            return Ok(ReadAction::NeedsFlush);
        }
        Ok(ReadAction::Continue)
    }

    // ===== Header =====
    pub fn handle_header(&mut self, frame: Headers) -> Result<(), ProtoError> {
        self.streams.recv_header(frame)
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

    // ==== Reset =====
    pub fn recv_reset(&mut self, frame: Reset) -> Result<(), ProtoError> {
        todo!()
    }

    // ===== Polling =====
    pub fn poll(&mut self, cx: &mut Context) -> Poll<Result<(), ProtoError>> {
        let span = self.span.clone();
        let _e = span.enter();
        loop {
            match self.state {
                ConnectionState::Open => {
                    let result = match self.poll2(cx) {
                        Poll::Ready(result) => result,
                        Poll::Pending => {
                            ////
                            return Poll::Pending;
                        }
                    };
                }
                ConnectionState::Closing(reason, initiator) => todo!(),
                ConnectionState::Closed(reason, initiator) => todo!(),
            }
        }
    }

    fn poll2(&mut self, cx: &mut Context) -> Poll<Result<(), ProtoError>> {
        // This happens outside of the loop to prevent needing to do a clock
        // check and then comparison of the queue possibly multiple times a
        // second (and thus, the clock wouldn't have changed enough to matter).
        self.clear_expired_reset_streams();

        loop {
            /* TODO: GOAWAY LOGIC */

            /* TODO:
             * ready!(self.poll_ready(cx))?;
             * pending control frames*/

            // read a frame
            match ready!(Pin::new(&mut self.codec).poll_next(cx)?) {
                Some(frame) => {
                    let result = self.recv_frame(frame)?;
                    match result {
                        ReadAction::Continue => continue,
                        ReadAction::NeedsFlush => {
                            self.codec.flush(cx);
                            return Poll::Ready(Ok(()));
                        }
                    }
                }
                None => {
                    tracing::trace!("codec closed");
                    return Poll::Ready(Ok(()));
                }
            }
        }
    }

    fn poll_control_write(
        &mut self,
        cx: &mut Context,
    ) -> Poll<Result<(), ProtoError>> {
        // write pending pong
        // ping
        // settings
        // refusal
        Poll::Ready(Ok(()))
    }

    fn clear_expired_reset_streams(&mut self) {
        self.streams
            .clear_expired_reset_streams();
    }

    // ===== Test =====
    #[cfg(feature = "test-util")]
    pub fn read_frame(&mut self) -> Result<Frame, ProtoError> {
        self.codec.read_frame()
    }
}
