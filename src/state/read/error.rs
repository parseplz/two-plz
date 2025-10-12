use crate::proto::Error as ProtoError;
use thiserror::Error;

use crate::{codec::UserError, proto::Error};

#[derive(Debug, Error)]
pub enum StateError {
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
