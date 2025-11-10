use std::{cmp::Ordering, time::Duration};

use bytes::Bytes;
use http::HeaderMap;
use tracing::trace;

use crate::{
    DEFAULT_INITIAL_WINDOW_SIZE, Data, Headers, Reason, Settings, StreamId,
    frame,
    headers::{self, Pseudo},
    proto::{
        MAX_WINDOW_SIZE, ProtoError, WindowSize,
        config::ConnectionConfig,
        streams::{
            Counts, Store,
            buffer::Buffer,
            flow_control::FlowControl,
            store::{Key, Ptr, Queue},
            stream::{NextAccept, NextComplete, NextResetExpire, Stream},
        },
    },
    request::Request,
    role::{PollMessage, Role},
    stream_id::StreamIdOverflow,
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
pub(super) struct Recv {
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

    /// Streams that have pending window updates
    /// pending_window_updates: Queue<NextWindowUpdate>,
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
}

#[derive(Debug)]
pub(super) enum Event {
    Headers(PollMessage),
    Data(Bytes),
    Trailers(HeaderMap),
}

impl Recv {
    pub fn new(config: &ConnectionConfig, role: &Role) -> Recv {
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
            next_stream_id: Ok(role.peer_init_stream_id()),
            pending_accept: Queue::new(),
            pending_complete: Queue::new(),
            pending_reset_expired: Queue::new(),
            reset_duration: config.reset_stream_duration,
            is_push_enabled: false,
            is_extended_connect_protocol_enabled: false,
            refused: None,
        }
    }

    pub fn next_accept(&mut self, store: &mut Store) -> Option<Key> {
        self.pending_accept
            .pop(store)
            .map(|ptr| ptr.key())
    }

    // ===== Data =====
    pub fn recv_data(
        &mut self,
        frame: Data,
        stream: &mut Ptr,
        role: &Role,
    ) -> Result<(), ProtoError> {
        let size = frame.payload().len();
        // This should have been enforced at the codec::FramedRead layer, so
        // this is just a sanity check.
        assert!(size <= MAX_WINDOW_SIZE as usize);
        let size = size as WindowSize;
        let is_ignoring_frame = stream.state.is_local_error();

        // check if stream in recv data state
        if !is_ignoring_frame && !stream.state.is_recv_streaming() {
            // TODO: There are cases where this can be a stream error of
            // STREAM_CLOSED instead...
            // Receiving a DATA frame when not expecting one is a protocol
            // error.
            return Err(ProtoError::library_go_away(Reason::PROTOCOL_ERROR));
        }

        if is_ignoring_frame {
            return self.dec_connection_window(size);
        }

        self.dec_connection_window(size)?;
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

        // TODO: needed ?
        //if !stream.is_recv {
        //    self.release_connection_capacity(sz, &mut None);
        //    return Ok(());
        //}

        // update stream flow control
        stream
            .recv_flow
            .dec_window(size)
            .map_err(ProtoError::library_go_away)?;

        let event = Event::Data(frame.into_payload());

        // Push the frame onto the recv buffer
        stream
            .pending_recv
            .push_back(&mut self.buffer, event);

        if is_eos {
            // for server
            // move the streams from pending complete to pending_accept
            if role.is_server() {
                while let Some(mut stream) = self
                    .pending_complete
                    .pop_if(stream.store_mut(), |stream| {
                        stream.state.is_recv_streaming()
                    })
                {
                    self.pending_accept.push(&mut stream);
                }
            }
            // notify the client
            stream.notify_recv();
        }
        Ok(())
    }

    // ===== Headers =====
    /// Check if the headers frame is in right format and parse the headers
    ///
    /// The caller ensures that the frame represents headers and not trailers.
    pub fn recv_headers(
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

        if frame.is_over_size() {
            Self::check_frame_size(is_initial, &frame, stream, counts)?;
        }

        let stream_id = frame.stream_id();
        let is_eos = frame.is_end_stream();
        let (pseudo, fields) = frame.into_parts();

        // check extended protocol and response headers in request
        if !self.is_extended_protocol_usage_correct(stream, &pseudo, counts)
            && !Self::are_response_headers_in_request(stream, &pseudo, counts)
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
                .convert_poll_message(pseudo, fields, stream_id, None)?;

            // add headers to stream
            stream
                .pending_recv
                .push_back(&mut self.buffer, Event::Headers(message));

            if is_eos {
                // Only servers can receive a headers frame that initiates the
                // stream. This is verified in `Streams` before calling this
                // function.
                if counts.role().is_server() {
                    // Correctness: never push a stream to `pending_accept`
                    // without having the corresponding headers frame pushed to
                    // `stream.pending_recv`.
                    self.pending_accept.push(stream);
                } else {
                    // for client we notify the response has arrived
                    stream.notify_recv();
                }
            } else {
                // if not EOS, add to pending complete
                self.pending_complete.push(stream);
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
            let content_length = headers::parse_u64(
                    content_length.as_bytes(),
                )
                .map_err(|_| {
                    proto_err!(stream: "could not parse content-length| stream={:?}", stream.id);
                    ProtoError::library_reset(
                        stream.id,
                        Reason::PROTOCOL_ERROR,
                    )
                })?;

            stream.content_length = ContentLength::Remaining(content_length);
            // END_STREAM on headers frame with non-zero content-length is malformed.
            // https://datatracker.ietf.org/doc/html/rfc9113#section-8.1.1
            if frame.is_end_stream()
                && content_length > 0
                && frame
                    .pseudo()
                    .status
                    .is_none_or(|status| status != 204 && status != 304)
            {
                proto_err!(stream: "recv_headers with END_STREAM| content-length is not zero| stream={:?};", stream.id);
                return Err(ProtoError::library_reset(
                    stream.id,
                    Reason::PROTOCOL_ERROR,
                ));
            }
        }
        Ok(())
    }

    #[inline(always)]
    pub fn check_frame_size(
        is_initial: bool,
        frame: &frame::Headers,
        stream: &mut Ptr,
        counts: &mut Counts,
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
        tracing::debug!(
            "stream error REQUEST_HEADER_FIELDS_TOO_LARGE -- \
                recv_headers| frame is over size| stream={:?}",
            stream.id
        );
        // server
        if counts.role().is_server() && is_initial {
            let mut response = Headers::new(
                stream.id,
                headers::Pseudo::response(
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
        stream: &mut Ptr,
        pseudo: &Pseudo,
        counts: &mut Counts,
    ) -> bool {
        if pseudo.protocol.is_some()
            && counts.role().is_server()
            && !self.is_extended_connect_protocol_enabled
        {
            proto_err!(stream: "cannot use :protocol if extended connect protocol is disabled; stream={:?}", stream.id);
            false
        } else {
            true
        }
    }

    #[inline(always)]
    fn are_response_headers_in_request(
        stream: &mut Ptr,
        pseudo: &Pseudo,
        counts: &mut Counts,
    ) -> bool {
        if pseudo.status.is_some() && counts.role().is_server() {
            proto_err!(stream: "cannot use :status header for requests; stream={:?}", stream.id);
            false
        } else {
            true
        }
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
            proto_err!(conn: "id ({:?}) < next_id ({:?})", id, next_id);
            return Err(ProtoError::library_go_away(Reason::PROTOCOL_ERROR));
        }
        self.next_stream_id = id.next_id();

        if !counts.can_inc_num_recv_streams() {
            self.refused = Some(id);
            return Ok(None);
        }

        Ok(Some(id))
    }

    /// Transition the stream based on receiving trailers
    pub fn recv_trailers(
        &mut self,
        frame: Headers,
        stream: &mut Ptr,
        role: &Role,
    ) -> Result<(), ProtoError> {
        // Transition the state
        stream.state.recv_close()?;

        if stream
            .ensure_content_length_zero()
            .is_err()
        {
            proto_err!(stream: "recv_trailers| content-length is not zero| stream={:?};",  stream.id);
            return Err(ProtoError::library_reset(
                stream.id,
                Reason::PROTOCOL_ERROR,
            ));
        }

        stream
            .pending_recv
            .push_back(&mut self.buffer, Event::Trailers(frame.into_fields()));

        if role.is_server() {
            // for server
            // move the streams from pending complete to pending_accept
            while let Some(mut stream) = self
                .pending_complete
                .pop_if(stream.store_mut(), |stream| {
                    stream.state.is_recv_streaming()
                })
            {
                self.pending_accept.push(&mut stream);
            }
        }

        // since trailer is EOS we can notify client
        stream.notify_recv();
        Ok(())
    }

    // ===== Settings =====

    pub fn apply_local_settings(
        &mut self,
        settings: &Settings,
        store: &mut Store,
    ) -> Result<(), ProtoError> {
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
                    tracing::trace!("decrementing all windows; dec={}", dec);

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
                    tracing::trace!("incrementing all windows; inc={}", inc);
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

    /// Get the max ID of streams we can receive.
    ///
    /// This gets lowered if we send a GOAWAY frame.
    pub fn max_stream_id(&self) -> StreamId {
        self.max_stream_id
    }

    // ===== RESET =====
    pub fn recv_reset(
        &mut self,
        frame: frame::Reset,
        stream: &mut Stream,
        counts: &mut Counts,
    ) -> Result<(), ProtoError> {
        if counts.can_inc_num_remote_reset_streams() {
            counts.inc_num_remote_reset_streams();
        } else {
            tracing::warn!(
                "recv_reset; remotely-reset pending-accept streams reached limit ({:?})",
                counts.max_remote_reset_streams(),
            );
            return Err(ProtoError::library_go_away_data(
                Reason::ENHANCE_YOUR_CALM,
                "too_many_resets",
            ));
        }

        // Notify the stream
        stream
            .state
            .recv_reset(frame, stream.is_pending_send);

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
                "enqueue_reset_expiration| dropped {:?}, over max_concurrent_reset_streams",
                stream.id
            );
        }
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

    // ===== Misc ======
    pub fn next_stream_id(&self) -> Result<StreamId, ProtoError> {
        if let Ok(id) = self.next_stream_id {
            Ok(id)
        } else {
            Err(ProtoError::library_go_away(Reason::PROTOCOL_ERROR))
        }
    }

    pub fn init_window_sz(&self) -> WindowSize {
        self.init_stream_window_sz
    }

    pub fn take_request(&mut self, stream: &mut Ptr) -> Request {
        while let Some(event) = stream
            .pending_recv
            .pop_front(&mut self.buffer)
        {
            dbg!(event);
        }

        todo!()

        //match stream
        //    .pending_recv
        //    .pop_front(&mut self.buffer)
        //{
        //    Some(Event::Headers(Server(request))) => request,
        //    _ => {
        //        unreachable!("server stream queue must start with Headers")
        //    }
        //}
    }
}
