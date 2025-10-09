// PING (payload) => recvd
//                <= PING (payload) + ack
//
// Graceful shutdown
//      spec - 2 * GOAWAY
//      hyper - 1 * GOAWAY + PING + 1 * GOAWAY

use crate::frame::Ping;

#[derive(Debug, Default)]
pub struct PingPong {
    pending_ping: Option<Ping>,
    awaiting_pong: bool,
    pending_pong: Option<Ping>,
    awaiting_shutdown: bool,
}

#[derive(Debug)]
pub(crate) enum ReceivedPing {
    Ok,
    MustAck,
    Unknown,
    Shutdown,
}

impl PingPong {
    pub fn new() -> PingPong {
        PingPong::default()
    }

    fn handle(&mut self, frame: Ping) -> ReceivedPing {
        // ping frames (must respond with PONG)
        if !frame.is_ack() {
            self.pending_pong = Some(Ping::pong(frame.into_payload()));
            return ReceivedPing::MustAck;
        }

        if let Some(pending) = self.pending_ping.take() {
            if pending.payload() == frame.payload() {
                if self.awaiting_shutdown {
                    return ReceivedPing::Shutdown;
                } else {
                    return ReceivedPing::Ok;
                }
            }

            self.pending_ping = Some(pending);
        }

        ReceivedPing::Unknown
    }

    pub(crate) fn ping_shutdown(&mut self) {
        self.awaiting_shutdown = true;
        self.pending_ping = Some(Ping::new(Ping::SHUTDOWN));
    }

    pub fn send_ping(&mut self) {
        self.awaiting_pong = true;
        self.pending_ping = Some(Ping::new(Ping::USER));
    }

    pub(crate) fn pending_pong(&mut self) -> Option<Ping> {
        self.pending_pong.take()
    }

    pub(crate) fn pending_ping(&mut self) -> Option<Ping> {
        self.pending_ping.take()
    }
}
