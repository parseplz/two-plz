use crate::{
    frame::{Frame, Kind},
    proto::connection::Connection,
};
use thiserror::Error;
use tokio::io::AsyncRead;
use tracing::{debug, info};

use crate::proto::Error as ProtoError;

pub enum State<T, E> {
    ReadFrame(Connection<T, E>),
    ParseFrame(Connection<T, E>),
    ApplySettings(Connection<T, E>),
    Send(Connection<T, E>),
    End,
}

#[derive(Debug, Error)]
pub enum StateError {
    #[error("proto| {0}")]
    Proto(ProtoError),
}

impl<T, E> State<T, E>
where
    T: AsyncRead + Unpin,
{
    pub fn init(conn: Connection<T, E>) -> Self {
        State::ReadFrame(conn)
    }

    pub async fn next(mut self) -> Result<Self, StateError> {
        let next_state = match self {
            Self::ReadFrame(mut conn) => {
                let frame = conn
                    .read_frame()
                    .await
                    .map_err(StateError::Proto)?;
                debug!("[+] read frame| {:?}", frame.kind());
                conn.set_curr_frame(frame);
                Self::ParseFrame(conn)
            }
            Self::ParseFrame(mut conn) => {
                let frame = conn.curr_frame();
                match frame.kind() {
                    Kind::Data => todo!(),
                    Kind::Headers => todo!(),
                    Kind::Priority => todo!(),
                    Kind::Reset => todo!(),
                    Kind::Settings => todo!(),
                    Kind::PushPromise => todo!(),
                    Kind::Ping => todo!(),
                    Kind::GoAway => todo!(),
                    Kind::WindowUpdate => Self::ApplySettings(conn),
                    Kind::Continuation => todo!(),
                    Kind::Unknown => todo!(),
                }
            }
            Self::ApplySettings(mut conn) => {
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
