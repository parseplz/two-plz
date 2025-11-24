use bytes::Buf;
use bytes::Bytes;
use tokio::io::AsyncWrite;
use tracing::trace;

use crate::Codec;
use crate::codec::UserError;
use crate::frame;
use crate::frame::DEFAULT_INITIAL_WINDOW_SIZE;
use crate::frame::Frame;
use crate::frame::Reason;
use crate::proto::ProtoError;
use crate::proto::config::ConnectionConfig;
use crate::proto::error::Initiator;
use crate::proto::streams::Counts;
use crate::proto::streams::Resolve;
use crate::proto::streams::Store;
use crate::proto::streams::buffer::Buffer;
use crate::proto::streams::flow_control::FlowControl;
use crate::proto::streams::flow_control::Window;
use crate::proto::streams::send_buffer::SendBuffer;
use crate::proto::streams::store::Ptr;
use crate::proto::streams::store::Queue;
use crate::proto::streams::stream;
use std::cmp::Ordering;
use std::cmp::min;
use std::io;
use std::task::Context;
use std::task::Poll;
use std::task::Waker;

use crate::{
    frame::StreamId, frame::StreamIdOverflow, proto::WindowSize, role::Role,
};

#[derive(Debug)]
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
    pub next_stream_id: Result<StreamId, StreamIdOverflow>,

    /// Queue of streams waiting for socket capacity to send a frame.
    pending_send: Queue<stream::NextSend>,

    /// Queue of streams waiting for window capacity to produce data.
    pending_capacity: Queue<stream::NextSendCapacity>,

    /// Queue of streams waiting for capacity due to max concurrency
    pending_open: Queue<stream::NextOpen>,

    /// Queue of streams waiting to be reset
    pending_reset: Queue<stream::NextResetExpire>,

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
            next_stream_id: Ok(role.peer_init_stream_id()),
            pending_capacity: Queue::new(),
            pending_open: Queue::new(),
            pending_reset: Queue::new(),
            pending_send: Queue::new(),
            is_push_enabled: false,
            is_extended_connect_protocol_enabled: false,
        }
    }

    /// Queue a frame to be sent to the remote
    pub fn queue_frame(
        &mut self,
        frame: Frame,
        buffer: &mut Buffer<Frame<Bytes>>,
        stream: &mut Ptr,
        task: &mut Option<Waker>,
    ) {
        stream
            .pending_send
            .push_back(buffer, frame);
        self.schedule_send(stream, task);
    }

    /// Clear the send queue for a stream
    pub fn clear_queue<B>(
        &mut self,
        buffer: &mut Buffer<Frame<B>>,
        stream: &mut Ptr,
    ) {
        while let Some(frame) = stream.pending_send.pop_front(buffer) {
            tracing::trace!(?frame, "dropping");
        }
    }

    /// Schedule a stream to be sent
    pub fn schedule_send(
        &mut self,
        stream: &mut Ptr,
        task: &mut Option<Waker>,
    ) {
        // If the stream is waiting to be opened, nothing more to do.
        if stream.is_send_ready() {
            tracing::trace!(?stream.id, "schedule_send");
            // Queue the stream
            self.pending_send.push(stream);

            if let Some(task) = task.take() {
                task.wake();
            }
        }
    }

    // ===== Headers =====
    pub fn send_headers<B>(
        &mut self,
        frame: frame::Headers,
        buffer: &mut Buffer<Frame<B>>,
        stream: &mut Ptr,
        counts: &mut Counts,
        task: &mut Option<Waker>,
    ) -> Result<(), UserError> {
        Self::check_headers(frame.fields())?;
        let end_stream = frame.is_end_stream();

        // Update the state
        stream.state.send_open(end_stream)?;

        let mut pending_open = false;
        if counts
            .role()
            .is_local_init(frame.stream_id())
        // TODO
        //&& !stream.is_pending_push
        {
            pending_open = true;
            self.pending_open.push(stream);
        }

        // Queue the frame for sending
        //
        // This call expects that, since new streams are in the open queue, new
        // streams won't be pushed on pending_send.
        stream
            .pending_send
            .push_back(buffer, frame.into());

        // Need to notify the connection when pushing onto pending_open since
        // queue_frame only notifies for pending_send.
        if pending_open && let Some(task) = task.take() {
            task.wake();
        }
        Ok(())
    }

    // ===== settings =====
    pub fn apply_remote_settings<B>(
        &mut self,
        settings: &frame::Settings,
        buffer: &mut Buffer<Frame<B>>,
        store: &mut Store,
        counts: &mut Counts,
        task: &mut Option<Waker>,
    ) -> Result<(), super::ProtoError> {
        if let Some(val) = settings.is_push_enabled() {
            self.is_push_enabled = val
        }

        if let Some(val) = settings.is_extended_connect_protocol_enabled() {
            self.is_extended_connect_protocol_enabled = val;
        }

        if let Some(new) = settings.initial_window_size() {
            let old = self.init_stream_window_sz;
            self.init_stream_window_sz = new;

            match new.cmp(&old) {
                Ordering::Less => {
                    // decrease the (remote) window on every open stream.
                    let dec = old - new;
                    let mut total_reclaimed = 0;
                    store.try_for_each(|mut stream| {
                        let stream = &mut *stream;
                        if stream.state.is_send_closed()
                        // TODO
                        // && stream.buffered_send_data == 0 {
                        {
                            return Ok(());
                        }
                        stream
                            .send_flow
                            .dec_window(dec)
                            .map_err(ProtoError::library_go_away)?;

                        // It's possible that decreasing the window causes
                        // `window_size` (the stream-specific window) to fall
                        // below `allocated` (the portion of the
                        // connection-level window that we have allocated to
                        // the stream). In this case, we should take that
                        // excess allocation away and reassign it to other
                        // streams.
                        let current_send_flow = stream.send_flow.available();
                        let allocated = stream.connection_window_allocated;
                        if allocated > current_send_flow {
                            let extra = allocated - current_send_flow;
                            total_reclaimed += extra;
                            stream.connection_window_allocated -= extra;
                        }
                        Ok::<_, ProtoError>(())
                    })?;
                    if total_reclaimed > 0 {
                        self.assign_connection_capacity(
                            total_reclaimed,
                            store,
                            counts,
                        );
                    }
                }
                Ordering::Greater => {
                    let inc = new - old;
                    store.try_for_each(|mut stream| {
                        self.recv_stream_window_update(&mut stream, inc)
                            .map_err(ProtoError::library_go_away)
                    })?;
                }
                Ordering::Equal => (),
            }
        }

        Ok(())
    }

    // ===== window update =====
    pub fn recv_connection_window_update(
        &mut self,
        inc: WindowSize,
        store: &mut Store,
        counts: &mut Counts,
    ) -> Result<(), Reason> {
        self.flow.inc_window(inc)?;
        self.assign_connection_capacity(inc, store, counts);
        Ok(())
    }

    pub fn assign_connection_capacity<R>(
        &mut self,
        inc: WindowSize,
        store: &mut R,
        counts: &mut Counts,
    ) where
        R: Resolve,
    {
        while self.flow.available() > 0 {
            let stream = match self.pending_capacity.pop(store) {
                Some(stream) => stream,
                None => return,
            };

            // Streams pending capacity may have been reset before capacity
            // became available. In that case, the stream won't want any
            // capacity, and so we shouldn't "transition" on it, but just evict
            // it and continue the loop.
            // TODO
            //if !(stream.state.is_send_streaming() || stream.buffered_send_data > 0) {
            if !stream.state.is_send_streaming() {
                continue;
            }

            counts.transition(stream, |_, stream| {
                // Try to assign capacity to the stream. This will also
                // re-queue the stream if there isn't enough connection level
                // capacity to fulfill the capacity request.
                self.try_assign_capacity(stream);
            })
        }
    }

    fn try_assign_capacity(&mut self, stream: &mut Ptr) {
        let stream_available = stream.send_flow.available();

        // check stream capacity
        // when, connection window update
        if stream_available == 0 {
            return;
        }

        // check connection capacity on
        // 1. stream window update
        // 2. settings initial frame size change
        if self.flow.available() == 0 {
            self.pending_capacity.push(stream);
            return;
        }

        //     | connection_flow_available
        // min | stream_flow_available
        //     | remaining_data_len
        let needed = min(
            min(self.flow.available(), stream_available),
            stream.remaining_data_len as u32,
        );

        // no flow is needed, possibly empty data frame ?
        if needed == 0 {
            self.pending_send.push(stream);
            return;
        }

        // increase stream allocated connection window
        stream.connection_window_allocated += needed;
        // reduce connection window
        self.flow.dec_window(needed);
    }

    pub fn recv_stream_window_update(
        &mut self,
        stream: &mut Ptr,
        inc: WindowSize,
    ) -> Result<(), Reason> {
        if stream.state.is_send_closed() {
            return Ok(());
        }
        stream.send_flow.inc_window(inc)?;
        self.try_assign_capacity(stream);
        Ok(())
    }

    fn reclaim_all_capacity(
        &mut self,
        stream: &mut Ptr<'_>,
        counts: &mut Counts,
    ) {
        let allocated = stream.connection_window_allocated;
        if allocated > 0 {
            self.assign_connection_capacity(allocated, stream, counts);
        }
    }

    // ===== reset =====
    pub fn send_reset(
        &mut self,
        reason: Reason,
        initiator: Initiator,
        stream: &mut Ptr,
        buffer: &mut Buffer<Frame<Bytes>>,
        counts: &mut Counts,
        task: &mut Option<Waker>,
    ) {
        let is_reset = stream.state.is_reset();
        let is_closed = stream.state.is_closed();
        let is_empty = stream.pending_send.is_empty();
        let stream_id = stream.id;

        if is_reset {
            // Don't double reset
            tracing::trace!(
                " -> not sending RST_STREAM ({:?} is already reset)",
                stream_id
            );
            return;
        }

        // Transition the state to reset no matter what.
        stream.set_reset(reason, initiator);

        // If closed AND the send queue is flushed, then the stream cannot be
        // reset explicitly, either. Implicit resets can still be queued.
        if is_closed && is_empty {
            tracing::trace!(
                " -> not sending explicit RST_STREAM ({:?} was closed \
                 and send queue was flushed)",
                stream_id
            );
            return;
        }

        // Clear all pending outbound frames.
        self.clear_queue(buffer, stream);

        // add reset to the send queue
        let frame = frame::Reset::new(stream.id, reason);
        self.queue_frame(frame.into(), buffer, stream, task);
        self.reclaim_all_capacity(stream, counts);
    }

    pub fn schedule_implicit_reset(
        &mut self,
        stream: &mut Ptr,
        reason: Reason,
        counts: &mut Counts,
        task: &mut Option<Waker>,
    ) {
        if stream.state.is_closed() {
            // Stream is already closed, nothing more to do
            return;
        }
        stream.state.set_scheduled_reset(reason);
        self.reclaim_all_capacity(stream, counts);
        self.schedule_send(stream, task);
    }

    pub fn handle_error<B>(
        &mut self,
        buffer: &mut Buffer<Frame<B>>,
        stream: &mut Ptr,
        counts: &mut Counts,
    ) {
        self.clear_queue(buffer, stream);
        self.reclaim_all_capacity(stream, counts);
    }

    // ===== Misc ====
    pub fn init_window_sz(&self) -> WindowSize {
        self.init_stream_window_sz
    }

    fn check_headers(fields: &http::HeaderMap) -> Result<(), UserError> {
        // 8.1.2.2. Connection-Specific Header Fields
        if fields.contains_key(http::header::CONNECTION)
            || fields.contains_key(http::header::TRANSFER_ENCODING)
            || fields.contains_key(http::header::UPGRADE)
            || fields.contains_key("keep-alive")
            || fields.contains_key("proxy-connection")
        {
            tracing::debug!("illegal connection-specific headers found");
            return Err(UserError::MalformedHeaders);
        } else if let Some(te) = fields.get(http::header::TE)
            && te != "trailers"
        {
            tracing::debug!("illegal connection-specific headers found");
            return Err(UserError::MalformedHeaders);
        }
        Ok(())
    }

    // ===== polling =====

    fn pop_pending_open<'s>(
        &mut self,
        store: &'s mut Store,
        counts: &mut Counts,
    ) -> Option<Ptr<'s>> {
        // check for any pending open streams
        if counts.can_inc_num_send_streams() {
            if let Some(mut stream) = self.pending_open.pop(store) {
                trace!("pop pending open| {:?}", stream.id);
                counts.inc_num_send_streams(&mut stream);
                // TODO
                // stream.notify_send();
                return Some(stream);
            }
        }
        None
    }

    fn pop_frame(
        &mut self,
        buffer: &mut Buffer<Frame<Bytes>>,
        store: &mut Store,
        max_frame_size: usize,
        counts: &mut Counts,
    ) -> Option<Frame<Bytes>> {
        loop {
            match self.pending_send.pop(store) {
                Some(mut stream) => {
                    // It's possible that this stream, besides having data to
                    // send, is also queued to send a reset, and thus is
                    // already in the queue to wait for "some time" after a
                    // reset.
                    //
                    // To be safe, we just always ask the stream.
                    let is_pending_reset =
                        stream.is_pending_reset_expiration();

                    let frame = match stream.pending_send.pop_front(buffer) {
                        Some(Frame::Data(mut frame)) => {
                            // Get the amount of capacity remaining for stream's
                            // window.
                            let stream_available =
                                stream.send_flow.available();

                            // Zero length data frames always have capacity to
                            // be sent.
                            if stream.remaining_data_len > 0
                                && stream_available == 0
                            {
                                // Ensure that the stream is waiting for
                                // connection level capacity
                                //
                                // TODO: uncomment
                                // debug_assert!(stream.is_pending_send_capacity);

                                // The stream has no more capacity, this can
                                // happen if the remote reduced the stream
                                // window. In this case, we need to buffer the
                                // frame and wait for a window update...
                                stream
                                    .pending_send
                                    .push_front(buffer, frame.into());
                                continue;
                            }

                            //     | remaining_data_len
                            // min | max_frame_size
                            //     | stream flow available
                            let len = min(
                                min(stream.remaining_data_len, max_frame_size),
                                stream_available as usize,
                            );

                            // There *must* be be enough connection level
                            // capacity at this point.
                            debug_assert!(
                                len <= self.flow.available() as usize
                            );
                            // consume
                            //      - stream flow control
                            //      - connection window allocated
                            //      - remaining_data_len
                            let _res = stream.send_flow.dec_window(len as u32);
                            stream.connection_window_allocated -= len as u32;
                            stream.remaining_data_len -= len;

                            // split the buf
                            let data = frame.payload_mut().split_to(len);
                            let mut data_frame =
                                frame::Data::new(stream.id, data);
                            let eos = stream.remaining_data_len > 0;
                            data_frame.set_end_stream(eos);
                            Frame::Data(data_frame)
                        }
                        Some(frame) => frame.map(|_| {
                            unreachable!(
                                "Frame::map closure will only be called \
                                 on DATA frames."
                            )
                        }),
                        None => {
                            if let Some(reason) =
                                stream.state.get_scheduled_reset()
                            {
                                stream.set_reset(reason, Initiator::Library);

                                let frame =
                                    frame::Reset::new(stream.id, reason);
                                Frame::Reset(frame)
                            } else {
                                // If the stream receives a RESET from the
                                // peer, it may have had data buffered to be
                                // sent, but all the frames are cleared in
                                // clear_queue(). Instead of doing O(N)
                                // traversal through queue to remove, lets just
                                // ignore the stream here.
                                debug_assert!(stream.state.is_closed());
                                counts.transition_after(
                                    stream,
                                    is_pending_reset,
                                );
                                continue;
                            }
                        }
                    };
                    if stream.state.is_idle() {
                        self.last_opened_id = stream.id;
                    }
                    if !stream.pending_send.is_empty()
                        || stream.state.is_scheduled_reset()
                    {
                        // TODO: Only requeue the sender IF it is ready to send
                        // the next frame. i.e. don't requeue it if the next
                        // frame is a data frame and the stream does not have
                        // any more capacity.
                        self.pending_send.push(&mut stream);
                    }
                    counts.transition_after(stream, is_pending_reset);
                    return Some(frame);
                }
                None => return None,
            }
        }
    }

    pub fn poll_complete<T>(
        &mut self,
        cx: &mut Context,
        buffer: &mut Buffer<Frame<Bytes>>,
        store: &mut Store,
        counts: &mut Counts,
        dst: &mut Codec<T, Bytes>,
    ) -> Poll<io::Result<()>>
    where
        T: AsyncWrite + Unpin,
    {
        ready!(dst.poll_ready(cx))?;
        let max_frame_len = dst.max_send_frame_size();
        loop {
            if let Some(mut stream) = self.pop_pending_open(store, counts) {
                self.pending_send
                    .push_front(&mut stream);
                self.try_assign_capacity(&mut stream);
            }

            match self.pop_frame(buffer, store, max_frame_len, counts) {
                Some(frame) => {
                    dst.buffer(frame)
                        .expect("invalid frame");
                    ready!(dst.poll_ready(cx))?;
                }
                None => {
                    ready!(dst.flush(cx))?;
                }
            }
        }

        return Poll::Ready(Ok(()));
    }
}
