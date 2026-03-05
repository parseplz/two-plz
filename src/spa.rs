use std::{
    io,
    task::{Context, Poll},
};

use bytes::Bytes;
use tokio::io::AsyncWrite;
use tracing::{error, trace};

use crate::{
    Codec,
    frame::{Frame, StreamId},
    proto::{
        PingPayload,
        ping_pong::PingHandler,
        streams::{Buffer, Store},
    },
};

#[derive(Clone, Debug)]
pub struct SpaTracker {
    pub(crate) pending_spa: Vec<StreamId>,
    // pending streams with stream level window update
    pending_stream_window_update: Option<Vec<StreamId>>,
    pub(crate) mode: Mode,
}

impl From<Mode> for SpaTracker {
    fn from(mode: Mode) -> Self {
        Self {
            pending_spa: Vec::new(),
            pending_stream_window_update: None,
            mode,
        }
    }
}

impl SpaTracker {
    pub fn are_pending_stream_window_update(&self) -> bool {
        self.pending_stream_window_update
            .is_some()
    }

    pub fn add_pending_spa(&mut self, stream: StreamId) {
        self.pending_spa.push(stream)
    }

    pub fn add_pending_stream_update(&mut self, stream: StreamId) {
        let queue = self
            .pending_stream_window_update
            .get_or_insert_with(Vec::new);
        queue.push(stream)
    }

    pub fn remove_pending_stream_update(&mut self, stream: StreamId) {
        self.pending_stream_window_update = self
            .pending_stream_window_update
            .take()
            .and_then(|mut q| {
                q.retain(|x| *x != stream);
                (!q.is_empty()).then_some(q)
            });
    }

    pub(crate) fn add_enhanced_ping_first_ping(
        &mut self,
        ping_handler: &mut PingHandler,
    ) {
        if let Mode::EnhancedPing(state, payload_sent) = &mut self.mode {
            let payload = build_ping_payload();
            ping_handler.send_ping(payload);
            *state = PingState::FirstSent;
            *payload_sent = Some(payload);
            trace!("first ping sent");
        }
    }

    pub fn is_enhanced_ping(&self) -> bool {
        matches!(self.mode, Mode::EnhancedPing(..))
    }

    pub fn recvd_ping(&mut self, cx: &mut Context, payload: &PingPayload) {
        match &mut self.mode {
            Mode::Ping(Some((sent, ack))) => {
                if sent == payload {
                    *ack = true
                }
            }
            Mode::EnhancedPing(state, mayb_payload) => match state {
                PingState::FirstSent => {
                    if *mayb_payload == Some(*payload) {
                        trace!("first pong recvd");
                        *state = PingState::SecondToSend;
                        *mayb_payload = None;
                    }
                }
                PingState::SecondSent => {
                    if *mayb_payload == Some(*payload) {
                        trace!("second pong recvd");
                        *state = PingState::Fin;
                        *mayb_payload = None;
                        cx.waker().wake_by_ref();
                    }
                }
                _ => (),
            },
            Mode::Native | Mode::Ping(None) => (),
        }
    }

    // ===== polling =====
    pub fn poll_fill_frames<T>(
        &mut self,
        cx: &mut Context,
        store: &mut Store,
        buffer: &mut Buffer<Frame<Bytes>>,
        dst: &mut Codec<T, Bytes>,
    ) -> Poll<io::Result<()>>
    where
        T: AsyncWrite + Unpin,
    {
        if !dst.write_buf_empty() {
            ready!(dst.flush(cx))?;
        }
        let size = self.pending_spa.len() * 10;
        dst.inc_write_buffer(size);
        self.pending_spa.sort();
        self.fill_frames(store, buffer, dst);
        dst.flush(cx)
    }

    pub fn fill_frames<T>(
        &self,
        store: &mut Store,
        buffer: &mut Buffer<Frame<Bytes>>,
        dst: &mut Codec<T, Bytes>,
    ) where
        T: AsyncWrite + Unpin,
    {
        tracing::trace!("filling frames");
        for id in self.pending_spa.iter() {
            let mut stream = store.find_mut(id).unwrap();
            let mut frame = stream
                .pending_send
                .pop_front(buffer)
                .unwrap();
            if let Frame::Data(ref mut data) = frame {
                stream.state.send_close();
                data.set_end_stream(true);
            } else {
                error!("not a data frame| {:?}", frame.kind())
            }
            let _ = dst.buffer(frame);
        }
    }

    pub fn poll<T>(
        &mut self,
        cx: &mut Context,
        buffer: &mut Buffer<Frame<Bytes>>,
        store: &mut Store,
        dst: &mut Codec<T, Bytes>,
        ping_handler: &mut PingHandler,
        can_perform_spa: bool,
        is_done: &mut bool,
    ) -> Poll<io::Result<()>>
    where
        T: AsyncWrite + Unpin,
    {
        *is_done = false;
        match &mut self.mode {
            Mode::Native
            | Mode::Ping(Some((_, true)))
            | Mode::EnhancedPing(PingState::Fin, _) => {
                ready!(self.poll_fill_frames(cx, store, buffer, dst))?;
                *is_done = true;
                Poll::Ready(Ok(()))
            }
            Mode::Ping(None) => {
                tracing::trace!("ping payload queued");
                let payload = build_ping_payload();
                ping_handler.send_ping(payload);
                self.mode = Mode::Ping(Some((payload, false)));
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            Mode::EnhancedPing(
                state @ PingState::SecondToSend,
                mayb_payload,
            ) if can_perform_spa => {
                tracing::trace!("second ping sent");
                *state = PingState::SecondSent;
                let payload = build_ping_payload();
                ping_handler.send_ping(payload);
                *mayb_payload = Some(payload);
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            Mode::Ping(Some((_, false))) | Mode::EnhancedPing(..) => {
                Poll::Pending
            }
        }
    }
}

#[derive(Clone, Default, Debug, PartialEq)]
pub enum Mode {
    #[default]
    Native,
    Ping(Option<(PingPayload, bool)>),
    EnhancedPing(PingState, Option<PingPayload>),
}

#[derive(Clone, Default, Debug, PartialEq)]
pub enum PingState {
    #[default]
    Init,
    FirstSent,
    SecondToSend,
    SecondSent,
    Fin,
}

impl Mode {
    pub fn ping() -> Self {
        Self::Ping(None)
    }

    pub fn enhanced() -> Self {
        Self::EnhancedPing(PingState::Init, None)
    }
}

//pub(crate) fn build_ping_payload() -> PingPayload {
//    let mut payload = frame::Ping::SHUTDOWN;
//    let ptr: *const usize = value as *const usize;
//    let bytes = (ptr as usize).to_be_bytes();
//    let len = bytes.len();
//    payload[..len].copy_from_slice(&bytes);
//    payload
//}

pub(crate) fn build_ping_payload() -> PingPayload {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    let mut x = nanos;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    x.to_be_bytes()
}
