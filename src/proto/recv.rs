use std::{cmp::Ordering, time::Duration};

use http::HeaderMap;

use crate::{
    DEFAULT_INITIAL_WINDOW_SIZE, Reason, Settings, StreamId, frame, headers,
    proto::{
        self, ProtoError, WindowSize,
        buffer::Buffer,
        config::ConnectionConfig,
        count::Counts,
        flow_control::FlowControl,
        store::{Ptr, Queue, Store},
        stream::{NextAccept, NextResetExpire, Stream},
    },
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
    Body,
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
            next_stream_id: Ok(role.init_stream_id()),
            pending_accept: Queue::new(),
            pending_reset_expired: Queue::new(),
            reset_duration: config.reset_stream_duration,
            is_push_enabled: false,
            is_extended_connect_protocol_enabled: false,
            refused: None,
        }
    }

    // ===== Headers =====

    /// Transition the stream state based on receiving headers
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
            if frame.stream_id() > self.last_processed_id {
                self.last_processed_id = frame.stream_id();
            }

            // Increment the number of concurrent streams
            counts.inc_num_recv_streams(stream);
        }

        if !stream.content_length.is_head() {
            use super::stream::ContentLength;
            use http::header;

            if let Some(content_length) = frame
                .fields()
                .get(header::CONTENT_LENGTH)
            {
                let content_length = match headers::parse_u64(
                    content_length.as_bytes(),
                ) {
                    Ok(v) => v,
                    Err(_) => {
                        proto_err!(stream: "could not parse content-length; stream={:?}", stream.id);
                        return Err(ProtoError::library_reset(
                            stream.id,
                            Reason::PROTOCOL_ERROR,
                        )
                        .into());
                    }
                };

                stream.content_length =
                    ContentLength::Remaining(content_length);
                // END_STREAM on headers frame with non-zero content-length is malformed.
                // https://datatracker.ietf.org/doc/html/rfc9113#section-8.1.1
                if frame.is_end_stream()
                    && content_length > 0
                    && frame
                        .pseudo()
                        .status
                        .map_or(true, |status| status != 204 && status != 304)
                {
                    proto_err!(stream: "recv_headers with END_STREAM: content-length is not zero; stream={:?};", stream.id);
                    return Err(ProtoError::library_reset(
                        stream.id,
                        Reason::PROTOCOL_ERROR,
                    )
                    .into());
                }
            }
        }

        if frame.is_over_size() {
            // A frame is over size if the decoded header block was bigger than
            // SETTINGS_MAX_HEADER_LIST_SIZE.
            //
            // > A server that receives a larger header block than it is willing
            // > to handle can send an HTTP 431 (Request Header Fields Too
            // > Large) status code [RFC6585]. A client can discard responses
            // > that it cannot process.
            //
            // So, if peer is a server, we'll send a 431. In either case,
            // an error is recorded, which will send a REFUSED_STREAM,
            // since we don't want any of the data frames either.
            tracing::debug!(
                "stream error REQUEST_HEADER_FIELDS_TOO_LARGE -- \
                 recv_headers: frame is over size; stream={:?}",
                stream.id
            );
            return if counts.role().is_server() && is_initial {
                let mut res = frame::Headers::new(
                    stream.id,
                    headers::Pseudo::response(
                        ::http::StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE,
                    ),
                    HeaderMap::new(),
                );
                res.set_end_stream();
                Err(RecvHeaderBlockError::Oversize(Some(res)))
            } else {
                Err(RecvHeaderBlockError::Oversize(None))
            };
        }

        let stream_id = frame.stream_id();
        let (pseudo, fields) = frame.into_parts();

        if pseudo.protocol.is_some()
            && counts.role().is_server()
            && !self.is_extended_connect_protocol_enabled
        {
            proto_err!(stream: "cannot use :protocol if extended connect protocol is disabled; stream={:?}", stream.id);
            return Err(ProtoError::library_reset(
                stream.id,
                Reason::PROTOCOL_ERROR,
            )
            .into());
        }

        if pseudo.status.is_some() && counts.role().is_server() {
            proto_err!(stream: "cannot use :status header for requests; stream={:?}", stream.id);
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

            stream
                .pending_recv
                .push_back(&mut self.buffer, Event::Headers(message));

            // Only servers can receive a headers frame that initiates the stream.
            // This is verified in `Streams` before calling this function.
            if counts.role().is_server() {
                // Correctness: never push a stream to `pending_accept` without having the
                // corresponding headers frame pushed to `stream.pending_recv`.
                self.pending_accept.push(stream);
            }
        }

        Ok(())
    }

    /// Update state reflecting a new, remotely opened stream
    ///
    /// Returns the stream state if successful. `None` if refused
    pub fn open(
        &mut self,
        id: StreamId,
        mode: Open,
        counts: &mut Counts,
        peer: &Role,
    ) -> Result<Option<StreamId>, ProtoError> {
        // TODO: WHY ?
        //assert!(self.refused.is_none());

        peer.ensure_can_open(id, mode)?;

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
    ) -> Result<(), ProtoError> {
        // Transition the state
        stream.state.recv_close()?;

        if stream
            .ensure_content_length_zero()
            .is_err()
        {
            proto_err!(stream: "recv_trailers: content-length is not zero; stream={:?};",  stream.id);
            return Err(ProtoError::library_reset(
                stream.id,
                Reason::PROTOCOL_ERROR,
            ));
        }

        stream
            .pending_recv
            .push_back(&mut self.buffer, Event::Trailers(frame.into_fields()));
        Ok(())
    }

    // ===== Settings =====

    pub fn apply_local_settings(
        &mut self,
        settings: &Settings,
        store: &mut Store,
    ) -> Result<(), proto::ProtoError> {
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
                            .map_err(proto::ProtoError::library_go_away)?;
                        Ok::<_, proto::ProtoError>(())
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
                            .map_err(proto::ProtoError::library_go_away)?;
                        Ok::<_, proto::ProtoError>(())
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
    // ===== Misc ======
    pub fn next_stream_id(&self) -> Result<StreamId, ProtoError> {
        if let Ok(id) = self.next_stream_id {
            Ok(id)
        } else {
            Err(ProtoError::library_go_away(Reason::PROTOCOL_ERROR))
        }
    }

    // ===== Misc ====
    pub fn init_window_sz(&self) -> WindowSize {
        self.init_stream_window_sz
    }
}
