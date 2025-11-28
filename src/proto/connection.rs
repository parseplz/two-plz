use std::pin::Pin;
use std::task::Context;
use std::task::Poll;

use crate::frame;
use crate::frame::Reason;
use crate::proto::ProtoError;
use crate::proto::error::Initiator;
use crate::proto::go_away::GoAway;
use crate::proto::settings::SettingsAction;
use crate::proto::settings::SettingsHandler;
use crate::proto::streams::streams::Streams;
use crate::proto::streams::streams_ref::StreamRef;
use crate::role::Role;

use bytes::Bytes;
use futures::Stream;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::frame::{Headers, Settings};
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

pub struct Connection<T> {
    pub codec: Codec<T, Bytes>,
    pub streams: Streams<Bytes>,
    goaway_handler: GoAway,
    ping_handler: PingHandler,
    settings_handler: SettingsHandler,
    role: Role,
    state: ConnectionState,
    span: tracing::Span,
    /// An error to report back once complete.
    ///
    /// This exists separately from State in order to support
    /// graceful shutdown.
    error: Option<frame::GoAway>,
}

impl<T> Connection<T>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    pub fn new(
        role: Role,
        config: ConnectionConfig,
        codec: Codec<T, Bytes>,
    ) -> Self {
        Connection {
            state: ConnectionState::Open,
            goaway_handler: GoAway::new(),
            ping_handler: PingHandler::new(),
            settings_handler: SettingsHandler::new(
                config.local_settings.clone(),
            ),
            codec,
            role: role.clone(),
            span: tracing::debug_span!("connection| "),
            streams: Streams::new(role, config),
            error: None,
        }
    }

    // ===== Server =====
    pub fn next_accept(&mut self) -> Option<StreamRef<Bytes>> {
        self.streams.next_accept()
    }

    // ===== Codec =====
    pub fn buffer(&mut self, item: Frame<Bytes>) -> Result<(), UserError> {
        self.codec.buffer(item)
    }

    // ======== FRAMES ============
    pub fn recv_frame(&mut self, frame: Frame) -> Result<(), ProtoError> {
        match frame {
            Frame::Data(data) => self.streams.recv_data(data),
            Frame::Headers(headers) => self.streams.recv_header(headers),
            Frame::Priority(priority) => todo!(),
            Frame::Reset(reset) => self.streams.recv_reset(reset),
            Frame::Settings(settings) => self.recv_settings(settings),
            Frame::PushPromise(push_promise) => todo!(),
            Frame::Ping(ping) => {
                let action = self.ping_handler.handle(ping);
                // TODO
                //if action.is_shutdown() {
                //    todo!()
                //}
                Ok(())
            }
            Frame::GoAway(go_away) => {
                // This should prevent starting new streams,
                // but should allow continuing to process current streams
                // until they are all EOS. Once they are, State should
                // transition to GoAway.
                self.streams.recv_go_away(&go_away)?;
                self.error = Some(go_away);
                Ok(())
            }
            Frame::WindowUpdate(wupdate) => self.recv_window_update(wupdate),
        }?;
        Ok(())
    }

    // ===== Settings =====
    pub fn recv_settings(
        &mut self,
        local: Settings,
    ) -> Result<(), ProtoError> {
        if let SettingsAction::ApplyLocal(settings) =
            self.settings_handler.recv(local)?
        {
            self.apply_local_settings(settings)?;
        }
        Ok(())
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

    // ===== Ping =====
    pub fn recv_ping(&mut self, frame: Ping) -> PingAction {
        self.ping_handler.handle(frame)
    }

    pub fn pending_pong(&mut self) -> Option<Ping> {
        self.ping_handler.pending_pong()
    }

    // ==== Window Update =====
    fn recv_window_update(
        &mut self,
        window_update: frame::WindowUpdate,
    ) -> Result<(), ProtoError> {
        let id = window_update.stream_id();
        let size = window_update.size_increment();
        if id.is_zero() {
            self.streams
                .recv_connection_window_update(size)
        } else {
            self.streams
                .recv_stream_window_update(id, size)
        }
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
                ConnectionState::Closing(reason, initiator) => {
                    tracing::trace!("connection closing after flush");
                    ready!(self.codec.shutdown(cx))?;
                    self.state = ConnectionState::Closed(reason, initiator);
                }
                ConnectionState::Closed(reason, initiator) => {
                    return Poll::Ready(self.take_error(reason, initiator));
                }
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

    fn poll_control_frames(
        &mut self,
        cx: &mut Context,
    ) -> Poll<Result<(), ProtoError>> {
        // write pending pong
        // ping
        // settings
        ready!(self.poll_settings(cx))?;
        // refusal
        // window update
        ready!(self.poll_window_update(cx))?;
        Poll::Ready(Ok(()))
    }

    fn poll_settings(
        &mut self,
        cx: &mut Context,
    ) -> Poll<Result<(), ProtoError>> {
        ready!(
            self.settings_handler
                .poll_remote_settings(cx, &mut self.codec, &mut self.streams)
        )?;
        ready!(
            self.settings_handler
                .poll_local_settings(cx, &mut self.codec)
        )?;
        Poll::Ready(Ok(()))
    }

    fn poll_window_update(
        &mut self,
        cx: &mut Context,
    ) -> Poll<Result<(), ProtoError>> {
        ready!(
            self.streams
                .poll_window_update(cx, &mut self.codec)
        )?;
        Poll::Ready(Ok(()))
    }

    fn clear_expired_reset_streams(&mut self) {
        self.streams
            .clear_expired_reset_streams();
    }

    fn take_error(
        &mut self,
        ours: Reason,
        initiator: Initiator,
    ) -> Result<(), ProtoError> {
        let (debug_data, theirs) = self
            .error
            .take()
            .as_ref()
            .map_or((Bytes::new(), Reason::NO_ERROR), |frame| {
                (frame.debug_data().clone(), frame.reason())
            });

        match (ours, theirs) {
            (Reason::NO_ERROR, Reason::NO_ERROR) => Ok(()),
            (ours, Reason::NO_ERROR) => {
                Err(ProtoError::GoAway(Bytes::new(), ours, initiator))
            }
            // If both sides reported an error, give their
            // error back to th user. We assume our error
            // was a consequence of their error, and less
            // important.
            (_, theirs) => Err(ProtoError::remote_go_away(debug_data, theirs)),
        }
    }

    // ===== client misc ====
    /// Closes the connection by transitioning to a GOAWAY state
    /// if there are no streams or references
    pub fn maybe_close_connection_if_no_streams(&mut self) {
        // If we poll() and realize that there are no streams or references
        // then we can close the connection by transitioning to GOAWAY
        if !self
            .streams
            .has_streams_or_other_references()
        {
            self.go_away_now(Reason::NO_ERROR);
        }
    }

    /// Checks if there are any streams or references left
    pub fn has_streams_or_other_references(&self) -> bool {
        // If we poll() and realize that there are no streams or references
        // then we can close the connection by transitioning to GOAWAY
        self.streams
            .has_streams_or_other_references()
    }

    // ==== GOAWAY =====
    // send goaway - shutdown
    fn go_away(&mut self, id: StreamId, e: Reason) {
        let frame = frame::GoAway::new(id, e);
        self.streams.send_go_away(id);
        self.go_away_handler.go_away(frame);
    }

    // send goaway - immediate
    fn go_away_now(&mut self, e: Reason) {
        let last_processed_id = self.streams.last_processed_id();
        let frame = frame::GoAway::new(last_processed_id, e);
        self.go_away_handler.go_away_now(frame);
    }

    fn go_away_now_data(&mut self, e: Reason, data: Bytes) {
        let last_processed_id = self.streams.last_processed_id();
        let frame = frame::GoAway::with_debug_data(last_processed_id, e, data);
        self.go_away_handler.go_away_now(frame);
    }

    // TODO: implement method
    fn go_away_from_user(&mut self, e: Reason) {
        let last_processed_id = self.streams.last_processed_id();
        let frame = frame::GoAway::new(last_processed_id, e);
        self.go_away_handler
            .go_away_from_user(frame);
        // Notify all streams of reason we're abruptly closing.
        self.streams
            .handle_error(ProtoError::user_go_away(e));
    }

    // ===== Test =====
    #[cfg(feature = "test-util")]
    pub fn read_frame(&mut self) -> Result<Frame, ProtoError> {
        self.codec.read_frame()
    }
}
