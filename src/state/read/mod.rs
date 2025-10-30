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
pub fn read_runner<T, B>(
    conn: &mut Connection<T, B>,
    frame: Frame,
) -> Result<ReadState<T, B>, ReadError>
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

pub enum ReadState<'a, T, B> {
    HandleFrame(&'a mut Connection<T, B>, Frame),
    HandleData(&'a mut Connection<T, B>, Data),
    HandleHeaders(&'a mut Connection<T, B>, Headers),
    HandlePriority(&'a mut Connection<T, B>, Priority),
    HandleReset(&'a mut Connection<T, B>, Reset),
    HandleSettings(&'a mut Connection<T, B>, Settings),
    HandlePushPromise(&'a mut Connection<T, B>, PushPromise),
    HandlePing(&'a mut Connection<T, B>, Ping),
    HandleGoAway(&'a mut Connection<T, B>, GoAway),
    HandleWindowUpdate(&'a mut Connection<T, B>, WindowUpdate),
    NeedsFlush,
    End,
}

impl<'a, T, B> ReadState<'a, T, B>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    pub fn init(conn: &'a mut Connection<T, B>, frame: Frame) -> Self {
        ReadState::HandleFrame(conn, frame)
    }

    pub fn next(self) -> Result<Self, ReadError> {
        let next_state = match self {
            Self::HandleFrame(conn, frame) => match frame {
                Frame::Data(data) => Self::HandleData(conn, data),
                Frame::Priority(priority) => todo!(),
                Frame::Reset(reset) => todo!(),
                Frame::Settings(settings) => {
                    Self::HandleSettings(conn, settings)
                }
                Frame::PushPromise(push_promise) => todo!(),
                Frame::Ping(ping) => Self::HandlePing(conn, ping),
                Frame::GoAway(go_away) => todo!(),
                Frame::WindowUpdate(wu) => Self::HandleWindowUpdate(conn, wu),
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
            Self::HandleWindowUpdate(conn, window_update) => {
                let id = window_update.stream_id();
                let inc = window_update.size_increment();
                if id.is_zero() {
                    conn.recv_connection_window_update(inc)?
                } else {
                    conn.recv_stream_window_update(id, inc)?
                }
                todo!()
            }
            _ => todo!(),
        };
        Ok(next_state)
    }

    pub fn is_ended(&self) -> bool {
        matches!(self, Self::End) || matches!(self, Self::NeedsFlush)
    }
}

impl<'a, T, B> std::fmt::Debug for ReadState<'a, T, B> {
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
impl<'a, T, B> PartialEq for ReadState<'a, T, B> {
    fn eq(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}
