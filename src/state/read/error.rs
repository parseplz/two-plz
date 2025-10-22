use crate::proto::ProtoError;
use thiserror::Error;

use crate::codec::UserError;

#[derive(Debug, Error)]
pub enum ReadError {
    #[error("proto| {0}")]
    Proto(ProtoError),
    #[error("user| {0}")]
    User(#[from] UserError),
    // Ping
    #[error("pong pending")]
    PongPending,
    #[error("unknown ping")]
    UnknownPing,
}
