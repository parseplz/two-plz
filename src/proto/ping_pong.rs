use std::task::{Context, Poll};

use bytes::Buf;
use tokio::io::AsyncWrite;

use crate::{Codec, frame::Ping, proto::PingPayload};

// PING (payload) => recvd
//                <= PING (payload) + ack
//
// Graceful shutdown
//      spec - 2 * GOAWAY
//      hyper - 1 * GOAWAY + PING + 1 * GOAWAY

#[derive(Debug, Default)]
pub struct PingHandler {
    pending_ping: Option<PendingPing>,
    awaiting_pong: bool,
    pending_pong: Option<PingPayload>,
    awaiting_shutdown: bool,
}

#[derive(Debug)]
struct PendingPing {
    payload: PingPayload,
    sent: bool,
}

#[derive(Debug)]
pub enum PingAction {
    MustAck,
    Unknown,
    Shutdown,
}

impl PingAction {
    pub(crate) fn is_shutdown(&self) -> bool {
        matches!(*self, Self::Shutdown)
    }
}

impl PingHandler {
    pub fn new() -> PingHandler {
        PingHandler::default()
    }

    pub fn handle(&mut self, ping: Ping) -> PingAction {
        // ping frames (must respond with PONG)
        if !ping.is_ack() {
            self.pending_pong = Some(ping.into_payload());
            return PingAction::MustAck;
        }

        if let Some(pending) = self.pending_ping.take() {
            if &pending.payload == ping.payload() {
                assert_eq!(
                    &pending.payload,
                    &Ping::SHUTDOWN,
                    "pending_ping should be for shutdown",
                );
                tracing::trace!("recv PING SHUTDOWN ack");
                return PingAction::Shutdown;
            }

            // if not the payload we expected, put it back.
            self.pending_ping = Some(pending);
        }

        // else we were acked a ping we didn't send?
        // The spec doesn't require us to do anything about this,
        // so for resiliency, just ignore it for now.
        tracing::warn!("recv PING ack that we never sent| {:?}", ping);
        PingAction::Unknown
    }

    pub(crate) fn poll_pending<T, B>(
        &mut self,
        cx: &mut Context,
        dst: &mut Codec<T, B>,
    ) -> Poll<std::io::Result<()>>
    where
        T: AsyncWrite + Unpin,
        B: Buf,
    {
        if let Some(pong) = self.pending_pong.take() {
            if !dst.poll_ready(cx)?.is_ready() {
                self.pending_pong = Some(pong);
                return Poll::Pending;
            }

            dst.buffer(Ping::pong(pong).into())
                .expect("invalid pong frame");
        }
        if let Some(ref mut ping) = self.pending_ping
            && !ping.sent
        {
            if !dst.poll_ready(cx)?.is_ready() {
                return Poll::Pending;
            }

            dst.buffer(Ping::new(ping.payload).into())
                .expect("invalid ping frame");
            ping.sent = true;
        }
        Poll::Ready(Ok(()))
    }

    pub(crate) fn ping_shutdown(&mut self) {
        assert!(self.pending_ping.is_none());

        self.pending_ping = Some(PendingPing {
            payload: Ping::SHUTDOWN,
            sent: false,
        });
    }
}
