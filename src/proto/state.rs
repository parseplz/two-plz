use crate::{
    frame::{Frame, Kind, Ping},
    proto::connection::Connection,
};
use futures::StreamExt;
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncWrite};
use tracing::{debug, info};

use crate::proto::Error as ProtoError;

// E => Sent
// U => Received
async fn runner<T, E, U>(mut conn: Connection<T, E, U>)
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    tokio::select! {
        frame = conn.stream.next() => {
            match frame {
                Some(Ok(frame)) => {
                    let state_result = state_poller(&mut conn, frame);
                    todo!()
                }
                Some(Err(_)) => todo!(),
                None => todo!(),
            }
        }
        msg = conn.handler.receiver.recv() => {
            todo!()
        }
    }
}

fn state_poller<T, E, U>(
    conn: &mut Connection<T, E, U>,
    frame: Frame,
) -> Result<ReadState<T, E, U>, StateError> {
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
    End,
}

#[derive(Debug, Error)]
pub enum StateError {
    #[error("proto| {0}")]
    Proto(ProtoError),
}

impl<'a, T, E, U> ReadState<'a, T, E, U> {
    pub fn init(conn: &'a mut Connection<T, E, U>, frame: Frame) -> Self {
        ReadState::HandleFrame(conn, frame)
    }

    pub fn next(mut self) -> Result<Self, StateError> {
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
            Self::HandlePing(mut conn, ping) => {
                todo!()
            }
            _ => todo!(),
        };
        Ok(next_state)
    }

    pub fn is_ended(&self) -> bool {
        matches!(self, Self::End)
    }
}
