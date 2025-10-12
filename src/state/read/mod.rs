mod error;
use error::StateError;

use crate::{
    frame::{Frame, Ping},
    proto::{connection::Connection, ping_pong::PingAction},
};
use futures::StreamExt;
use tokio::io::{AsyncRead, AsyncWrite};


// Run the state machine once a frame is received
pub fn read_runner<T, E, U>(
    conn: &mut Connection<T, E, U>,
    frame: Frame,
) -> Result<ReadState<T, E, U>, StateError>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    let mut state = ReadState::init(conn, frame);
    loop {
        state = state.next()?;
        if state.is_ended() {
            break Ok(state);
        }
    }
}

pub enum ReadState<'a, T, E, U> {
    HandleFrame(&'a mut Connection<T, E, U>, Frame),
    HandlePing(&'a mut Connection<T, E, U>, Ping),
    NeedsFlush,
    End,
}

impl<'a, T, E, U> ReadState<'a, T, E, U>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    pub fn init(conn: &'a mut Connection<T, E, U>, frame: Frame) -> Self {
        ReadState::HandleFrame(conn, frame)
    }

    pub fn next(self) -> Result<Self, StateError> {
        let next_state = match self {
            Self::HandleFrame(conn, frame) => match frame {
                Frame::Data(data) => todo!(),
                Frame::Headers(headers) => todo!(),
                Frame::Priority(priority) => todo!(),
                Frame::PushPromise(push_promise) => todo!(),
                Frame::Settings(settings) => todo!(),
                Frame::Ping(ping) => Self::HandlePing(conn, ping),
                Frame::GoAway(go_away) => todo!(),
                Frame::WindowUpdate(window_update) => todo!(),
                Frame::Reset(reset) => todo!(),
            },
            Self::HandlePing(conn, ping) => match conn.handle_ping(ping) {
                PingAction::Ok => Self::End,
                PingAction::MustAck => {
                    if let Some(pong) = conn.ping_pong.pending_pong() {
                        conn.buffer(pong.into())?;
                        Self::NeedsFlush
                    } else {
                        return Err(StateError::PongPending);
                    }
                }
                PingAction::Unknown => {
                    return Err(StateError::UnknownPing);
                }
                PingAction::Shutdown => todo!(),
            },
            _ => todo!(),
        };
        Ok(next_state)
    }

    pub fn is_ended(&self) -> bool {
        matches!(self, Self::End)
        matches!(self, Self::End) || matches!(self, Self::NeedsFlush)
    }
}
    }
}
