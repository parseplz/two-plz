mod error;
use error::ReadError;

use crate::{
    frame::*,
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
) -> Result<ReadState<T, E, U>, ReadError>
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
    HandleData(&'a mut Connection<T, E, U>, Data),
    HandleHeaders(&'a mut Connection<T, E, U>, Headers),
    HandlePriority(&'a mut Connection<T, E, U>, Priority),
    HandleReset(&'a mut Connection<T, E, U>, Reset),
    HandleSettings(&'a mut Connection<T, E, U>, Settings),
    HandlePushPromise(&'a mut Connection<T, E, U>, PushPromise),
    HandlePing(&'a mut Connection<T, E, U>, Ping),
    HandleGoAway(&'a mut Connection<T, E, U>, GoAway),
    HandleWindowUpdate(&'a mut Connection<T, E, U>, WindowUpdate),
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

    pub fn next(self) -> Result<Self, ReadError> {
        let next_state = match self {
            Self::HandleFrame(conn, frame) => match frame {
                Frame::Data(data) => Self::HandleData(conn, data),
                Frame::Headers(headers) => todo!(),
                Frame::Priority(priority) => todo!(),
                Frame::Reset(reset) => todo!(),
                Frame::Settings(settings) => {
                    Self::HandleSettings(conn, settings)
                }
                Frame::PushPromise(push_promise) => todo!(),
                Frame::Ping(ping) => Self::HandlePing(conn, ping),
                Frame::GoAway(go_away) => todo!(),
                Frame::WindowUpdate(window_update) => todo!(),
            },
            Self::HandlePing(conn, ping) => match conn.handle_ping(ping) {
                PingAction::Ok => Self::End,
                PingAction::MustAck => {
                    if let Some(pong) = conn.pending_pong() {
                        conn.buffer(pong.into())?;
                        Self::NeedsFlush
                    } else {
                        return Err(ReadError::PongPending);
                    }
                }
                PingAction::Unknown => {
                    return Err(ReadError::UnknownPing);
                }
                PingAction::Shutdown => todo!(),
            },
            Self::HandleSettings(conn, settings) => {
                match conn.handle_settings(settings)? {
                    SettingsAction::SendAck => {
                        conn.buffer(Settings::ack().into())?;
                        let remote = conn.take_remote_settings();
                        // TODO: lead to further writes ?
                        conn.apply_remote_settings(remote)?;
                        Self::NeedsFlush
                    }
                    SettingsAction::ApplyLocal(settings) => {
                        // TODO: lead to further writes ?
                        conn.apply_local_settings(settings)?;
                        Self::End
                    }
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
            Self::HandleFrame(..) => write!(f, "HandleFrame"),
            Self::HandleData(..) => write!(f, "HandleData"),
            Self::HandleHeaders(..) => write!(f, "HandleHeaders"),
            Self::HandlePriority(..) => write!(f, "HandlePriority"),
            Self::HandleReset(..) => write!(f, "HandleReset"),
            Self::HandleSettings(..) => write!(f, "HandleSettings"),
            Self::HandlePushPromise(..) => write!(f, "HandlePushPromise"),
            Self::HandlePing(..) => write!(f, "HandlePing"),
            Self::HandleGoAway(..) => write!(f, "HandleGoAway"),
            Self::HandleWindowUpdate(..) => write!(f, "HandleWindowUpdate"),
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
