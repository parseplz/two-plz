use std::{
    cmp::Ordering,
    task::{Context, Poll},
    time::{Duration, Instant},
};

use bytes::{Bytes, BytesMut};
use http::HeaderMap;
use tracing::{Level, error, span, trace, warn};

use crate::{
    Codec,
    frame::{
        self, DEFAULT_INITIAL_WINDOW_SIZE, Reason, StreamId, StreamIdOverflow,
        headers::Pseudo,
    },
    message::{TwoTwo, request::Request, response::Response},
    proto::{
        MAX_WINDOW_SIZE, ProtoError, WindowSize,
        config::ConnectionConfig,
        streams::{
            Store,
            buffer::Buffer,
            counts::Counts,
            flow_control::FlowControl,
            store::{Key, Ptr, Queue},
            stream::{NextAccept, NextComplete, NextResetExpire, Stream},
        },
    },
    role::{PollMessage, Role},
};

#[derive(Debug)]
pub(super) enum RecvHeaderBlockError<T> {
    Oversize(T),
    State(ProtoError),
}

impl<T> From<ProtoError> for RecvHeaderBlockError<T> {
    fn from(err: ProtoError) -> Self {
        RecvHeaderBlockError::State(err)
    }
}

#[derive(Debug)]
pub(crate) enum Open {
    Headers,
    PushPromise,
}

impl Open {
    pub fn is_push_promise(&self) -> bool {
        matches!(*self, Self::PushPromise)
    }
}

#[derive(Debug)]
pub struct Recv {
    /// Holds frames that are waiting to be read
    buffer: Buffer<Event>,

    /// Connection level flow control governing received data
    flow: FlowControl,

    /// Initial window size of remote initiated streams
    init_stream_window_sz: WindowSize,

    /// The stream ID of the last processed stream
    last_processed_id: StreamId,

    /// Any streams with a higher ID are ignored.
    ///
    /// This starts as MAX, but is lowered when a GOAWAY is received.
    ///
    /// > After sending a GOAWAY frame, the sender can discard frames for
    /// > streams initiated by the receiver with identifiers higher than
    /// > the identified last stream.
    max_stream_id: StreamId,

    /// The lowest stream ID that is still idle
    pub next_stream_id: Result<StreamId, StreamIdOverflow>,

    /// New streams to be accepted
    pending_accept: Queue<NextAccept>,

    /// Streams waiting for EOS
    pending_complete: Queue<NextComplete>,

    /// Locally reset streams that should be reaped when they expire
    pending_reset_expired: Queue<NextResetExpire>,

    /// Refused StreamId, this represents a frame that must be sent out.
    refused: Option<StreamId>,

    /// How long locally reset streams should ignore received frames
    reset_duration: Duration,

    /// If push promises are allowed to be received.
    is_push_enabled: bool,

    /// If extended connect protocol is enabled.
    is_extended_connect_protocol_enabled: bool,

    /// Check whether connection window update needs to be sent
    pub check_connection_window_update: bool,

    /// Stream to check for sending pending window update
    pub check_stream_window_update: Option<Key>,
}

#[derive(Debug)]
pub(super) enum Event {
    Headers(PollMessage),
    Data(Bytes),
    Trailers(HeaderMap),
}

impl Recv {
    pub fn new(config: &ConnectionConfig, role: &Role) -> Recv {
        let next_stream_id = if role.is_server() {
            1
        } else {
            2
        };
        Recv {
            buffer: Buffer::new(),
            flow: FlowControl::new(
                config
                    .initial_connection_window_size
                    .unwrap_or(DEFAULT_INITIAL_WINDOW_SIZE),
            ),
            init_stream_window_sz: config
                .local_settings
                .initial_window_size()
                .unwrap_or(DEFAULT_INITIAL_WINDOW_SIZE),
            last_processed_id: StreamId::ZERO,
            max_stream_id: StreamId::MAX,
            next_stream_id: Ok(next_stream_id.into()),
            pending_accept: Queue::new(),
            pending_complete: Queue::new(),
            pending_reset_expired: Queue::new(),
            reset_duration: config.reset_stream_duration,
            is_push_enabled: false,
            is_extended_connect_protocol_enabled: false,
            refused: None,
            check_connection_window_update: false,
            check_stream_window_update: None,
        }
    }

    pub fn next_accept(&mut self, store: &mut Store) -> Option<Key> {
        self.pending_accept
            .pop(store)
            .map(|ptr| ptr.key())
    }

    // ===== Request =====
    pub fn take_request(&mut self, stream: &mut Ptr) -> Request {
        let mut request = match stream
            .pending_recv
            .pop_front(&mut self.buffer)
        {
            Some(Event::Headers(PollMessage::Server(request))) => request,
            _ => unreachable!("server stream queue must start with Headers"),
        };
        process_remaining_frames(&mut request, stream, &mut self.buffer);
        request
    }

    // ===== Data =====
    pub fn recv_data(
        &mut self,
        frame: frame::Data,
        stream: &mut Ptr,
        role: &Role,
    ) -> Result<(), ProtoError> {
        let size = frame.payload().len();
        // This should have been enforced at the codec::FramedRead layer, so
        // this is just a sanity check.
        assert!(size <= MAX_WINDOW_SIZE as usize);
        let size = size as WindowSize;
        // dec connection window
        self.dec_connection_window(size)?;
        // toggle check_connection_window_update
        self.check_connection_window_update = true;

        let is_ignoring_frame = stream.state.is_local_error();

        // check if stream in recv data state
        if !is_ignoring_frame && !stream.state.is_recv_streaming() {
            // TODO: There are cases where this can be a stream error of
            // STREAM_CLOSED instead...
            // Receiving a DATA frame when not expecting one is a protocol
            // error.
            error!("unexpected DATA frame| stream={:?}", stream.id);
            return Err(ProtoError::library_go_away(Reason::PROTOCOL_ERROR));
        }

        if is_ignoring_frame {
            trace!("ignoring");
            return Ok(());
        }

        if stream.recv_flow.window_size() < size {
            // http://httpwg.org/specs/rfc7540.html#WINDOW_UPDATE
            // > A receiver MAY respond with a stream error (Section 5.4.2) or
            // > connection error (Section 5.4.1) of type FLOW_CONTROL_ERROR if
            // > it is unable to accept a frame.
            //
            // So, for violating the **stream** window, we can send either a
            // stream or connection error. We've opted to send a stream
            // error.
            return Err(ProtoError::library_reset(
                stream.id,
                Reason::FLOW_CONTROL_ERROR,
            ));
        }

        // check if content length decrement causes underflow
        if stream
            .dec_content_length(frame.payload().len())
            .is_err()
        {
            return Err(ProtoError::library_reset(
                stream.id,
                Reason::PROTOCOL_ERROR,
            ));
        }

        let is_eos = frame.is_end_stream();

        // If EOS check if entire body is received and state transition cauess
        // err
        if is_eos {
            if stream
                .ensure_content_length_zero()
                .is_err()
            {
                return Err(ProtoError::library_reset(
                    stream.id,
                    Reason::PROTOCOL_ERROR,
                ));
            }

            if stream.state.recv_close().is_err() {
                return Err(ProtoError::library_go_away(
                    Reason::PROTOCOL_ERROR,
                ));
            }
        }

        // dec stream flow control
        stream
            .recv_flow
            .dec_window(size)
            .map_err(ProtoError::library_go_away)?;

        // Push the frame onto the recv buffer
        let event = Event::Data(frame.into_payload());
        stream
            .pending_recv
            .push_back(&mut self.buffer, event);

        if is_eos {
            self.move_from_pending_complete(stream, role);
        }

        // if !is_eos, add stream key to check for window update
        if !is_eos {
            self.check_stream_window_update = Some(stream.key);
        }
        trace!("to check wu| {:?}", stream.id);
        Ok(())
    }

    // ===== Headers =====
    /// Check if the headers frame is in right format and parse the headers
    ///
    /// The caller ensures that the frame represents headers and not trailers.
    pub(super) fn recv_headers(
        &mut self,
        frame: frame::Headers,
        stream: &mut Ptr,
        counts: &mut Counts,
    ) -> Result<(), RecvHeaderBlockError<Option<frame::Headers>>> {
        let is_initial = stream.state.recv_open(&frame)?;

        if is_initial {
            // save the id to use in GOAWAY frames
            if frame.stream_id() > self.last_processed_id {
                self.last_processed_id = frame.stream_id();
            }
            // Increment the number of concurrent streams
            counts.inc_num_recv_streams(stream);
        }

        // parse the content length
        if !stream.content_length.is_head() {
            Self::parse_content_length(stream, &frame)?;
        }

        let is_server = counts.role().is_server();

        if frame.is_over_size() {
            Self::ret_frame_oversize_err(is_initial, is_server, stream)?;
        }

        let stream_id = frame.stream_id();
        let is_eos = frame.is_end_stream();
        let (pseudo, fields) = frame.into_parts();

        // check extended protocol and response headers in request
        if !self.is_extended_protocol_usage_correct(&pseudo, is_server)
            && !Self::are_response_headers_in_request(&pseudo, is_server)
        {
            return Err(ProtoError::library_reset(
                stream.id,
                Reason::PROTOCOL_ERROR,
            )
            .into());
        }

        if !pseudo.is_informational() {
            let message = counts
                .role()
                .convert_poll_message(pseudo, fields, stream_id)?;

            // add headers to stream
            stream
                .pending_recv
                .push_back(&mut self.buffer, Event::Headers(message));

            if is_eos {
                self.move_from_pending_complete(stream, &role);
            }
        }

        Ok(())
    }

    #[inline(always)]
    pub fn parse_content_length(
        stream: &mut Ptr,
        frame: &frame::Headers,
    ) -> Result<(), ProtoError> {
        use super::stream::ContentLength;
        use http::header;

        if let Some(content_length) = frame
            .fields()
            .get(header::CONTENT_LENGTH)
        {
            let content_length = frame::headers::parse_u64(
                content_length.as_bytes(),
            )
            .map_err(|e| {
                error!("content-length parse| {:?}", e);
                ProtoError::library_reset(stream.id, Reason::PROTOCOL_ERROR)
            })?;

            stream.content_length =
                ContentLength::Remaining(content_length, content_length);
            // END_STREAM on headers frame with non-zero content-length is
            // malformed.
            // https://datatracker.ietf.org/doc/html/rfc9113#section-8.1.1
            if frame.is_end_stream()
                && content_length > 0
                && frame
                    .pseudo()
                    .status
                    .is_none_or(|status| status != 204 && status != 304)
            {
                error!("headers with END_STREAM| content-length is not zero");
                return Err(ProtoError::library_reset(
                    stream.id,
                    Reason::PROTOCOL_ERROR,
                ));
            }
        }
        Ok(())
    }

    #[inline(always)]
    fn ret_frame_oversize_err(
        is_initial: bool,
        is_server: bool,
        stream: &mut Ptr,
    ) -> Result<(), RecvHeaderBlockError<Option<frame::Headers>>> {
        // A frame is over size if the decoded header block was bigger than
        // SETTINGS_MAX_HEADER_LIST_SIZE.
        //
        // > A server that receives a larger header block than it is willing
        // > to handle can send an HTTP 431 (Request Header Fields Too
        // > Large) status code [RFC6585]. A client can discard responses
        // > that it cannot process.
        //
        // So, if role is a server, we'll send a 431. In either case,
        // an error is recorded, which will send a REFUSED_STREAM,
        // since we don't want any of the data frames either.
        error!("frame is over size",);
        // server
        if is_server && is_initial {
            let mut response = frame::Headers::new(
                stream.id,
                Pseudo::response(
                    ::http::StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE,
                ),
                HeaderMap::new(),
            );
            response.set_end_stream();
            Err(RecvHeaderBlockError::Oversize(Some(response)))
            // client
        } else {
            Err(RecvHeaderBlockError::Oversize(None))
        }
    }

    #[inline(always)]
    fn is_extended_protocol_usage_correct(
        &mut self,
        pseudo: &Pseudo,
        is_server: bool,
    ) -> bool {
        if pseudo.protocol.is_some()
            && is_server
            && !self.is_extended_connect_protocol_enabled
        {
            error!(
                "cannot use :protocol if extended connect protocol is \
                disabled"
            );
            false
        } else {
            true
        }
    }

    #[inline(always)]
    fn are_response_headers_in_request(
        pseudo: &Pseudo,
        is_server: bool,
    ) -> bool {
        if pseudo.status.is_some() && is_server {
            error!("cannot use :status header for requests");
            false
        } else {
            true
        }
    }

    /// Transition the stream based on receiving trailers
    pub fn recv_trailers(
        &mut self,
        frame: frame::Headers,
        stream: &mut Ptr,
        role: &Role,
    ) -> Result<(), ProtoError> {
        // Transition the state
        stream.state.recv_close()?;

        if stream
            .ensure_content_length_zero()
            .is_err()
        {
            error!("recv_trailers| content-length is not zero",);
            return Err(ProtoError::library_reset(
                stream.id,
                Reason::PROTOCOL_ERROR,
            ));
        }

        stream
            .pending_recv
            .push_back(&mut self.buffer, Event::Trailers(frame.into_fields()));
        self.move_from_pending_complete(stream, role);
        Ok(())
    }

    // ===== RESET =====
    pub fn recv_reset(
        &mut self,
        frame: frame::Reset,
        stream: &mut Stream,
        counts: &mut Counts,
    ) -> Result<(), ProtoError> {
        // Reseting a stream that the user hasn't accepted is possible,
        // but should be done with care. These streams will continue
        // to take up memory in the accept queue, but will no longer be
        // counted as "concurrent" streams.
        //
        // So, we have a separate limit for these.
        //
        // See https://github.com/hyperium/hyper/issues/2877
        if stream.is_pending_accept {
            if counts.can_inc_num_remote_reset_streams() {
                counts.inc_num_remote_reset_streams();
            } else {
                warn!(
                    "remotely-reset pending-accept streams reached limit| ({:?})",
                    counts.max_remote_reset_streams()
                );
                return Err(ProtoError::library_go_away_data(
                    Reason::ENHANCE_YOUR_CALM,
                    "too_many_resets",
                ));
            }
        }
        stream
            .state
            .recv_reset(frame, stream.is_pending_send);

        stream.notify_recv();

        // TODO: ws
        //stream.notify_push();
        //stream.notify_send();
        Ok(())
    }

    /// Add a locally reset stream to queue to be eventually reaped.
    pub fn enqueue_reset_expiration(
        &mut self,
        stream: &mut Ptr,
        counts: &mut Counts,
    ) {
        if !stream.state.is_local_error()
            || stream.is_pending_reset_expiration()
        {
            return;
        }

        if counts.can_inc_num_reset_streams() {
            counts.inc_num_reset_streams();
            trace!("enqueue_reset_expiration| added {:?}", stream.id);
            self.pending_reset_expired.push(stream);
        } else {
            trace!(
                "enqueue_reset_expiration| dropped {:?}, over \
                max_concurrent_reset_streams",
                stream.id
            );
        }
    }

    // ===== Settings =====
    pub fn apply_local_settings(
        &mut self,
        settings: &frame::Settings,
        store: &mut Store,
    ) -> Result<(), ProtoError> {
        let span = span!(Level::TRACE, "apply local settings| ");
        let _enter = span.enter();
        if let Some(val) = settings.is_extended_connect_protocol_enabled() {
            self.is_extended_connect_protocol_enabled = val;
        }

        if let Some(target) = settings.initial_window_size() {
            let old_sz = self.init_stream_window_sz;
            self.init_stream_window_sz = target;

            match target.cmp(&old_sz) {
                // We must decrease the (local) window on every open stream.
                Ordering::Less => {
                    let dec = old_sz - target;
                    trace!("decrementing| {}", dec);

                    store.try_for_each(|mut stream| {
                        stream
                            .recv_flow
                            .dec_window(dec)
                            .map_err(ProtoError::library_go_away)?;
                        Ok::<_, ProtoError>(())
                    })?;
                }
                // We must increase the (local) window on every open stream.
                Ordering::Greater => {
                    let inc = target - old_sz;
                    trace!("incrementing| {}", inc);
                    store.try_for_each(|mut stream| {
                        // XXX: Shouldn't the peer have already noticed our
                        // overflow and sent us a GOAWAY?
                        stream
                            .recv_flow
                            .inc_window(inc)
                            .map_err(ProtoError::library_go_away)?;
                        Ok::<_, ProtoError>(())
                    })?;
                }
                Ordering::Equal => (),
            }
        }
        Ok(())
    }

    // ===== GOAWAY =====
    pub fn go_away(&mut self, last_processed_id: StreamId) {
        assert!(self.max_stream_id >= last_processed_id);
        self.max_stream_id = last_processed_id;
    }

    /// Handle a connection-level error
    pub fn handle_error(&mut self, err: &ProtoError, stream: &mut Stream) {
        // Receive an error
        stream.state.handle_error(err);

        // TODO: partial messages
        //// clear pending recv queue
        if stream.is_pending_complete {
            while let Some(frame) = stream
                .pending_recv
                .pop_front(&mut self.buffer)
            {
                trace!(?frame, "dropping recvd frames");
            }
            stream.is_pending_complete = false;
        }

        // If a receiver is waiting, notify it
        stream.notify_recv();
        // TODO: ws
        //stream.notify_send();
        //stream.notify_push();
    }

    // ====== Window Update =====
    pub fn dec_connection_window(
        &mut self,
        size: WindowSize,
    ) -> Result<(), ProtoError> {
        if self.flow.window_size() < size {
            return Err(ProtoError::library_go_away(
                Reason::FLOW_CONTROL_ERROR,
            ));
        }
        self.flow
            .dec_window(size)
            .map_err(ProtoError::library_go_away)
    }

    pub fn inc_connection_window(
        &mut self,
        size: WindowSize,
    ) -> Result<(), ProtoError> {
        self.flow
            .inc_window(size)
            .map_err(ProtoError::library_go_away)
    }

    pub fn should_send_connection_window_update(&self) -> Option<WindowSize> {
        self.flow.should_send_window_update()
    }

    // ===== Misc ======

    pub fn init_window_sz(&self) -> WindowSize {
        self.init_stream_window_sz
    }

    pub fn last_processed_id(&self) -> StreamId {
        self.last_processed_id
    }

    /// check if the ID is the next expected and within the limit of total no
    /// of recv streams
    pub fn can_open(
        &mut self,
        id: StreamId,
        mode: Open,
        counts: &mut Counts,
        role: &Role,
    ) -> Result<Option<StreamId>, ProtoError> {
        assert!(self.refused.is_none());
        role.ensure_can_open(id, mode)?;
        let next_id = self.next_stream_id()?;
        if id < next_id {
            error!("id ({:?}) < next_id ({:?})", id, next_id);
            return Err(ProtoError::library_go_away(Reason::PROTOCOL_ERROR));
        }
        self.next_stream_id = id.next_id();

        if !counts.can_inc_num_recv_streams() {
            self.refused = Some(id);
            return Ok(None);
        }

        Ok(Some(id))
    }

    /// Get the max ID of streams we can receive.
    ///
    /// This gets lowered if we send a GOAWAY frame.
    pub fn max_stream_id(&self) -> StreamId {
        self.max_stream_id
    }

    pub fn next_stream_id(&self) -> Result<StreamId, ProtoError> {
        if let Ok(id) = self.next_stream_id {
            Ok(id)
        } else {
            Err(ProtoError::library_go_away(Reason::PROTOCOL_ERROR))
        }
    }

    pub fn maybe_reset_next_stream_id(&mut self, id: StreamId) {
        if let Ok(next_id) = self.next_stream_id {
            // !role::is_local_init should have been called beforehand
            debug_assert_eq!(
                id.is_server_initiated(),
                next_id.is_server_initiated()
            );
            if id >= next_id {
                self.next_stream_id = id.next_id();
            }
        }
    }

    // called only when EOS is received for a stream
    // server - move streams from pending_complete to pending_accept
    // client - pop pending_complete and notify the client for current stream
    fn move_from_pending_complete(&mut self, stream: &mut Ptr, role: &Role) {
        while let Some(mut stream) = self
            .pending_complete
            .pop_if(stream.store_mut(), |stream| {
                stream.state.is_recv_end_stream()
            })
        {
            if role.is_server() {
                trace!("moved to pending accept| {:?}", stream.id);
                self.pending_accept.push(&mut stream);
            }
        }
        if role.is_client() {
            trace!("notifying recv");
            stream.notify_recv();
        }
    }

    // ===== EOF =====
    pub fn recv_eof(&mut self, stream: &mut Stream) {
        stream.state.recv_eof();
        stream.notify_recv();
        // TODO: ws
        //stream.notify_send();
        //stream.notify_push();
    }

    // ===== Clear =====
    pub fn clear_queues(
        &mut self,
        clear_pending_accept: bool,
        store: &mut Store,
        counts: &mut Counts,
    ) {
        self.clear_all_reset_streams(store, counts);
        if clear_pending_accept {
            self.clear_all_pending_accept(store, counts);
        }
    }

    fn clear_all_reset_streams(
        &mut self,
        store: &mut Store,
        counts: &mut Counts,
    ) {
        while let Some(stream) = self.pending_reset_expired.pop(store) {
            counts.transition_after(stream, true);
        }
    }

    fn clear_all_pending_accept(
        &mut self,
        store: &mut Store,
        counts: &mut Counts,
    ) {
        while let Some(stream) = self.pending_accept.pop(store) {
            counts.transition_after(stream, false);
        }
    }

    pub fn clear_expired_reset_streams(
        &mut self,
        store: &mut Store,
        counts: &mut Counts,
    ) {
        if !self.pending_reset_expired.is_empty() {
            let now = Instant::now();
            let reset_duration = self.reset_duration;
            while let Some(stream) =
                self.pending_reset_expired
                    .pop_if(store, |stream| {
                        let reset_at = stream
                            .reset_at
                            .expect("reset_at must be set if in queue");
                        // rust-lang/rust#86470 tracks a bug in the standard
                        // library where `Instant` subtraction can panic
                        // (because, on some platforms, `Instant` isn't
                        // actually monotonic). We use a saturating operation
                        // to avoid this panic here.
                        now.saturating_duration_since(reset_at)
                            > reset_duration
                    })
            {
                trace!("clearing expired| {:?}", stream.id);
                counts.transition_after(stream, true);
            }
        }
    }

    // ===== Poll =====
    pub(crate) fn send_pending_refusal<T>(
        &mut self,
        cx: &mut Context<'_>,
        dst: &mut Codec<T, Bytes>,
    ) -> Poll<std::io::Result<()>>
    where
        T: tokio::io::AsyncWrite + Unpin,
    {
        if let Some(stream_id) = self.refused {
            ready!(dst.poll_ready(cx))?;
            let frame = frame::Reset::new(stream_id, Reason::REFUSED_STREAM);
            dst.buffer(frame.into())
                .expect("invalid RST_STREAM frame");
        }
        self.refused = None;
        Poll::Ready(Ok(()))
    }

    pub fn poll_response(
        &mut self,
        cx: &Context,
        stream: &mut Ptr,
    ) -> Poll<Result<Response, PartialResponse>> {
        if stream.state.is_recv_end_stream() {
            let mut response = take_response(stream, &mut self.buffer)?;
            process_remaining_frames(&mut response, stream, &mut self.buffer);
            Poll::Ready(Ok(response))
        } else {
            if let Err(e) = stream.state.ensure_recv_open() {
                let inner_result =
                    match take_response(stream, &mut self.buffer) {
                        Ok(partial) => {
                            PartialResponse::new(Some(partial), Some(e), None)
                        }
                        Err(mut partial_with_err) => {
                            partial_with_err.remote_err = Some(e);
                            partial_with_err
                        }
                    };
                return Poll::Ready(Err(inner_result));
            };
            stream.recv_task = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}

fn take_response(
    stream: &mut Ptr,
    buffer: &mut Buffer<Event>,
) -> Result<Response, PartialResponse> {
    let mut response = match stream.pending_recv.pop_front(buffer) {
        Some(Event::Headers(PollMessage::Client(response))) => response,
        Some(_) => {
            unreachable!("client stream queue must start with Headers")
        }
        None => {
            return Err(PartialResponse::empty_queue(stream.id));
        }
    };
    process_remaining_frames(&mut response, stream, buffer);
    Ok(response)
}

fn process_remaining_frames<T>(
    message: &mut TwoTwo<T>,
    stream: &mut Ptr,
    buffer: &mut Buffer<Event>,
) {
    let mut body: Option<BytesMut> = None;
    while let Some(event) = stream.pending_recv.pop_front(buffer) {
        match event {
            Event::Headers(_) => {
                unreachable!("header already popped")
            }
            Event::Data(data) => {
                let buf = body.get_or_insert_with(|| {
                    let capacity = stream
                        .content_length()
                        .map(|size| size as usize)
                        // assume atleast two data frames of same size
                        // are received
                        .unwrap_or_else(|| data.len() * 2);

                    BytesMut::with_capacity(capacity)
                });

                buf.reserve(data.len());
                buf.extend_from_slice(&data);
            }
            Event::Trailers(header_map) => {
                message.set_trailer(header_map);
            }
        }
    }
    message.set_body(body);
}

pub struct PartialResponse {
    response: Option<Response>,
    remote_err: Option<ProtoError>,
    local_err: Option<ProtoError>,
}

impl PartialResponse {
    fn new(
        response: Option<Response>,
        remote_err: Option<ProtoError>,
        local_err: Option<ProtoError>,
    ) -> Self {
        Self {
            response,
            remote_err,
            local_err,
        }
    }

    fn empty_queue(id: StreamId) -> Self {
        let err = ProtoError::library_reset(id, Reason::PROTOCOL_ERROR);
        Self::new(None, None, Some(err))
    }
}

impl std::fmt::Debug for PartialResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PartialResponse")
            .field("response", &self.response)
            .field("remote_err", &self.remote_err)
            .field("local_err", &self.local_err)
            .finish()
    }
}
