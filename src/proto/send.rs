use crate::DEFAULT_INITIAL_WINDOW_SIZE;
use crate::Frame;
use crate::Reason;
use crate::Settings;
use crate::proto::Error;
use crate::proto::buffer::Buffer;
use crate::proto::config::ConnectionConfig;
use crate::proto::store::Ptr;
use crate::proto::store::Queue;
use crate::proto::store::Store;
use crate::proto::stream;
use std::cmp::Ordering;

use crate::{
    StreamId,
    proto::{WindowSize, flow_control::FlowControl},
    role::Role,
    stream_id::StreamIdOverflow,
};

pub struct Send {
    /// Initial window size of locally initiated streams
    init_stream_window_sz: WindowSize,

    /// connection level
    flow: FlowControl,

    /// Stream ID of the last stream opened.
    last_opened_id: StreamId,

    /// Any streams with a higher ID are ignored.
    ///
    /// This starts as MAX, but is lowered when a GOAWAY is received.
    ///
    /// > After sending a GOAWAY frame, the sender can discard frames for
    /// > streams initiated by the receiver with identifiers higher than
    /// > the identified last stream.
    max_stream_id: StreamId,

    // TODO: make this configurable
    // hyper Builder::StreamId
    /// Stream identifier to use for next initialized stream.
    next_stream_id: Result<StreamId, StreamIdOverflow>,

    /// Queue of streams waiting for socket capacity to send a frame.
    pending_send: Queue<stream::NextSend>,

    /// Queue of streams waiting for window capacity to produce data.
    pending_capacity: Queue<stream::NextSendCapacity>,

    /// Queue of streams waiting for capacity due to max concurrency
    pending_open: Queue<stream::NextOpen>,

    /// Queue of streams waiting to be reset
    pending_reset: Queue<stream::NextResetExpire>,
    // TODO
    //is_push_enabled: bool,
    //is_extended_connect_protocol_enabled: bool,

    is_push_enabled: bool,
    is_extended_connect_protocol_enabled: bool,
}

impl Send {
    pub fn new(config: &ConnectionConfig, role: &Role) -> Send {
        Send {
            flow: FlowControl::new(DEFAULT_INITIAL_WINDOW_SIZE),
            init_stream_window_sz: config
                .peer_settings
                .initial_window_size()
                .unwrap_or(DEFAULT_INITIAL_WINDOW_SIZE),
            last_opened_id: StreamId::ZERO,
            max_stream_id: StreamId::MAX,
            next_stream_id: Ok(role.init_stream_id()),
            pending_capacity: Queue::new(),
            pending_open: Queue::new(),
            pending_reset: Queue::new(),
            pending_send: Queue::new(),
            is_push_enabled: false,
            is_extended_connect_protocol_enabled: false,
        }
    }

    /// Queue a frame to be sent to the remote
    pub fn queue_frame<B>(
        &mut self,
        frame: Frame<B>,
        buffer: &mut Buffer<Frame<B>>,
        stream: &mut Ptr,
    ) {
        // Queue the frame in the buffer
        stream
            .pending_send
            .push_back(buffer, frame);
        self.schedule_send(stream);
    }

    pub fn schedule_send(&mut self, stream: &mut Ptr) {
        // If the stream is waiting to be opened, nothing more to do.
        if stream.is_send_ready() {
            tracing::trace!(?stream.id, "schedule_send");
            // Queue the stream
            self.pending_send.push(stream);
        }
    }
    /// settings
    pub fn apply_remote_settings(
        &mut self,
        settings: &Settings,
        store: &mut Store,
    ) -> Result<(), super::Error> {
        if let Some(val) = settings.is_push_enabled() {
            self.is_push_enabled = val
        }

        if let Some(val) = settings.is_extended_connect_protocol_enabled() {
            self.is_extended_connect_protocol_enabled = val;
        }

        if let Some(val) = settings.initial_window_size() {
            let old_val = self.init_stream_window_sz;
            self.init_stream_window_sz = val;

            match val.cmp(&old_val) {
                Ordering::Less => {
                    let dec = old_val - val;
                    store.try_for_each(|mut stream| {
                        let stream = &mut *stream;
                        if stream.state.is_send_closed() {
                            return Ok(());
                        }
                        stream
                            .send_flow
                            .dec_window(dec)
                            .map_err(Error::library_go_away)
                    })?
                }
                Ordering::Greater => {
                    let inc = val - old_val;
                    store.try_for_each(|mut stream| {
                        self.recv_stream_window_update(inc, &mut stream)
                            .map_err(Error::library_go_away)
                    })?;
                }
                Ordering::Equal => (),
            }
        }

        Ok(())
    }
    pub fn recv_stream_window_update(
        &mut self,
        inc: WindowSize,
        stream: &mut Ptr,
    ) -> Result<(), Reason> {
        if stream.state.is_send_closed() {
            return Ok(());
        }
        stream.send_flow.inc_window(inc)?;
        Ok(())
    }
}
