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
            span: tracing::debug_span!("conn", "{}", role.as_str()),
            streams: Streams::new(role, config),
            error: None,
        }
    }

    // ===== Server =====
    pub fn next_accept(&mut self) -> Option<StreamRef<Bytes>> {
        self.streams.next_accept()
    }

    // ===== CLIENT =====
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

    // ===== COMMON =====
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
                if action.is_shutdown() {
                    let last_processed_id = self.streams.last_processed_id();
                    self.go_away_graceful(last_processed_id, Reason::NO_ERROR);
                }
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
            Frame::WindowUpdate(wupdate) => {
                self.streams.recv_window_update(wupdate)
            }
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

    // ===== POLLING =====
    pub fn poll(&mut self, cx: &mut Context) -> Poll<Result<(), ProtoError>> {
        let span = self.span.clone();
        let _e = span.enter();
        loop {
            match self.state {
                ConnectionState::Open => {
                    let result = match self.poll2(cx) {
                        Poll::Ready(result) => result,
                        Poll::Pending => {
                            ready!(
                                self.streams
                                    .poll_complete(cx, &mut self.codec)
                            )?;
                            if self.can_go_away() {
                                self.go_away_now(Reason::NO_ERROR);
                                continue;
                            }
                            return Poll::Pending;
                        }
                    };
                    self.handle_poll2_result(result)?
                }
                ConnectionState::Closing(reason, initiator) => {
                    tracing::trace!("closing after flush");
                    ready!(self.codec.shutdown(cx))?;
                    self.state = ConnectionState::Closed(reason, initiator);
                }
                ConnectionState::Closed(reason, initiator) => {
                    return Poll::Ready(self.take_error(reason, initiator));
                }
            }
        }
    }

    #[inline(always)]
    fn can_go_away(&self) -> bool {
        (self.error.is_some()
            || self
                .go_away_handler
                .should_close_on_idle())
            && !self.streams.has_streams()
    }

    fn poll2(&mut self, cx: &mut Context) -> Poll<Result<(), ProtoError>> {
        // This happens outside of the loop to prevent needing to do a clock
        // check and then comparison of the queue possibly multiple times a
        // second (and thus, the clock wouldn't have changed enough to matter).
        self.streams
            .clear_expired_reset_streams();

        loop {
            // First, ensure that the `Connection` is able to receive a frame
            //
            // The order here matters:
            // - poll_go_away may buffer a graceful shutdown GOAWAY frame
            // - If it has, we've also added a PING to be sent in poll_ready
            if let Some(reason) = ready!(self.poll_go_away(cx)?) {
                if self.goaway_handler.should_close_now() {
                    if self.goaway_handler.is_user_initiated() {
                        // A user initiated abrupt shutdown shouldn't return
                        // the same error back to the user.
                        return Poll::Ready(Ok(()));
                    } else {
                        return Poll::Ready(Err(ProtoError::library_go_away(
                            reason,
                        )));
                    }
                }
                // Only NO_ERROR should be waiting for idle
                debug_assert_eq!(
                    reason,
                    Reason::NO_ERROR,
                    "graceful GOAWAY should be NO_ERROR"
                );
            }
            ready!(self.poll_control_frames(cx))?;
            // read a frame
            match ready!(Pin::new(&mut self.codec).poll_next(cx)?) {
                Some(frame) => {
                    self.recv_frame(frame)?;
                }
                None => {
                    trace!("codec closed");
                    self.streams
                        .recv_eof(false)
                        .expect("mutex poisoned");
                    return Poll::Ready(Ok(()));
                }
            }
        }
    }

    /// Send any pending GOAWAY frames.
    ///
    /// This will return `Some(reason)` if the connection should be closed
    /// afterwards. If this is a graceful shutdown, this returns `None`.
    fn poll_go_away(
        &mut self,
        cx: &mut Context,
    ) -> Poll<Option<std::io::Result<Reason>>> {
        self.goaway_handler
            .send_pending_go_away(cx, &mut self.codec)
    }

    fn handle_poll2_result(
        &mut self,
        result: Result<(), ProtoError>,
    ) -> Result<(), ProtoError> {
        match result {
            // The connection has shutdown normally
            Ok(()) => {
                self.state = ConnectionState::Closing(
                    Reason::NO_ERROR,
                    Initiator::Library,
                );
                Ok(())
            }
            // Attempting to read a frame resulted in a connection level
            // error. This is handled by setting a GOAWAY frame followed by
            // terminating the connection.
            Err(ProtoError::GoAway(debug_data, reason, initiator)) => {
                self.poll2_go_away(reason, debug_data, initiator);
                Ok(())
            }
            // Attempting to read a frame resulted in a stream level error.
            // This is handled by resetting the frame then trying to read
            // another frame.
            Err(ProtoError::Reset(id, reason, initiator)) => {
                debug_assert_eq!(initiator, Initiator::Library);
                match self.streams.send_reset(id, reason) {
                    Ok(()) => (),
                    // only possible - Too many internal resets,
                    //                 ENHANCE_YOUR_CALM
                    Err(crate::proto::error::GoAway {
                        debug_data,
                        reason,
                    }) => {
                        self.poll2_go_away(
                            reason,
                            debug_data,
                            Initiator::Library,
                        );
                    }
                }
                Ok(())
            }
            // Attempting to read a frame resulted in an I/O error. All
            // active streams must be reset.
            //
            // TODO: Are I/O errors recoverable?
            Err(ProtoError::Io(kind, inner)) => {
                let e = ProtoError::Io(kind, inner);
                // Reset and Notify all active streams
                self.streams.handle_error(e.clone());
                // Some client implementations drop the connections without
                // notifying its peer Attempting to read after the client
                // dropped the connection results in UnexpectedEof If as a
                // server, we don't have anything more to send, just close the
                // connection without error
                //
                // See https://github.com/hyperium/hyper/issues/3427
                if self.streams.send_buffer_is_empty()
                    && matches!(kind, std::io::ErrorKind::UnexpectedEof)
                    && (self.role.is_server()
                        || self
                            .error
                            .as_ref()
                            .map(|f| f.reason() == Reason::NO_ERROR)
                            == Some(true))
                {
                    self.state = ConnectionState::Closed(
                        Reason::NO_ERROR,
                        Initiator::Library,
                    );
                    return Ok(());
                }
                Err(e)
            }
        }
    }

    fn poll2_go_away(
        &mut self,
        reason: Reason,
        debug_data: Bytes,
        initiator: Initiator,
    ) {
        let e = ProtoError::GoAway(debug_data.clone(), reason, initiator);
        // We may have already sent a GOAWAY for this error,
        // if so, don't send another, just flush and close up.
        if self
            .goaway_handler
            .going_away()
            .map_or(false, |frame| frame.reason() == reason)
        {
            self.state = ConnectionState::Closing(reason, initiator);
            return;
        }

        // Reset and Notify all active streams
        self.streams.handle_error(e);
        self.go_away_now_data(reason, debug_data);
    }

    fn poll_control_frames(
        &mut self,
        cx: &mut Context,
    ) -> Poll<Result<(), ProtoError>> {
        let span = tracing::trace_span!("control frames");
        let _e = span.enter();
        ready!(
            self.ping_handler
                .poll_pending(cx, &mut self.codec)
        )?;
        ready!(
            self.settings_handler
                .poll_remote_settings(cx, &mut self.codec, &mut self.streams)
        )?;
        ready!(
            self.settings_handler
                .poll_local_settings(cx, &mut self.codec)
        )?;
        ready!(
            self.streams
                .send_pending_refusal(cx, &mut self.codec)
        )?;
        ready!(
            self.streams
                .poll_window_update(cx, &mut self.codec)
        )?;
        Poll::Ready(Ok(()))
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
    // TODO: Expose Api
    fn go_away_graceful(&mut self, id: StreamId, e: Reason) {
        let frame = frame::GoAway::new(id, e);
        // sets recv.last_processed_id
        self.streams.send_go_away(id);
        self.goaway_handler
            .enqueue_go_away(frame);
    }

    // send goaway - immediate
    fn go_away_now(&mut self, e: Reason) {
        let last_processed_id = self.streams.last_processed_id();
        let frame = frame::GoAway::new(last_processed_id, e);
        self.goaway_handler.go_away_now(frame);
    }

    // send_goaway - immediate with data
    fn go_away_now_data(&mut self, e: Reason, data: Bytes) {
        let last_processed_id = self.streams.last_processed_id();
        let frame = frame::GoAway::with_debug_data(last_processed_id, e, data);
        self.goaway_handler.go_away_now(frame);
    }

    // TODO Expose Api
    fn go_away_from_user(&mut self, e: Reason) {
        let last_processed_id = self.streams.last_processed_id();
        let frame = frame::GoAway::new(last_processed_id, e);
        self.goaway_handler
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

impl<T> Drop for Connection<T> {
    fn drop(&mut self) {
        // Ignore errors as this indicates that the mutex is poisoned.
        let _ = self.streams.recv_eof(true);
    }
}
