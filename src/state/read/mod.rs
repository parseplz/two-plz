mod error;
use error::StateError;

use crate::{
    Settings,
    frame::{Frame, Ping},
    proto::{
        connection::Connection, ping_pong::PingAction,
        settings::SettingsAction,
    },
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
    HandleSettings(&'a mut Connection<T, E, U>, Settings),
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
                Frame::Settings(settings) => {
                    Self::HandleSettings(conn, settings)
                }
                Frame::Ping(ping) => Self::HandlePing(conn, ping),
                Frame::GoAway(go_away) => todo!(),
                Frame::WindowUpdate(window_update) => todo!(),
                Frame::Reset(reset) => todo!(),
            },
            Self::HandlePing(conn, ping) => match conn.handle_ping(ping) {
                PingAction::Ok => Self::End,
                PingAction::MustAck => {
                    if let Some(pong) = conn.pending_pong() {
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
            Self::HandleSettings(conn, settings) => {
                match conn.handle_settings(settings) {
                    Ok(SettingsAction::SendAck) => {
                        conn.buffer(Settings::ack().into())?;
                        let remote = conn.take_remote_settings();
                        // TODO: lead to further writes ?
                        conn.apply_remote_settings(remote);
                        Self::NeedsFlush
                    }
                    Ok(SettingsAction::ApplyLocal(settings)) => {
                        conn.apply_local_settings(settings);
                        // TODO: lead to further writes ?
                        Self::End
                    }
                    Err(e) => return Err(StateError::Proto(e)),
                }
            }
            _ => todo!(),
        };
        Ok(next_state)
    }

    pub fn is_ended(&self) -> bool {
        matches!(self, Self::End) || matches!(self, Self::NeedsFlush)
    }
}

impl<'a, T, E, U> std::fmt::Debug for ReadState<'a, T, E, U> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::HandleFrame(_, _) => write!(f, "HandleFrame"),
            Self::HandleSettings(_, _) => write!(f, "HandleSettings"),
            Self::HandlePing(_, _) => write!(f, "HandlePing"),
            Self::NeedsFlush => write!(f, "NeedsFlush"),
            Self::End => write!(f, "End"),
        }
    }
}

#[cfg(feature = "test-util")]
impl<'a, T, E, U> PartialEq for ReadState<'a, T, E, U> {
    fn eq(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}
