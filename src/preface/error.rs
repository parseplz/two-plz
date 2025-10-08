use thiserror::Error;

use crate::{codec::UserError, frame, proto};

#[derive(Debug, Error)]
#[error("preface error: {state}: {err}")]
pub struct PrefaceError {
    state: PrefaceErrorState,
    err: PrefaceErrorKind,
}

impl PrefaceError {
    pub fn new(state: PrefaceErrorState, err: PrefaceErrorKind) -> Self {
        Self {
            state,
            err,
        }
    }
}

impl From<PrefaceErrorKind> for PrefaceError {
    fn from(err: PrefaceErrorKind) -> Self {
        PrefaceError::new(PrefaceErrorState::ReadPreface, err)
    }
}

#[derive(Debug, Error)]
pub enum PrefaceErrorKind {
    // server
    #[error("write preface received")]
    InvalidPreface,
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    // common
    #[error("user: {0}")]
    User(#[from] UserError),
    #[error("proto")]
    Proto,
    #[error("eof")]
    Eof,
    #[error("wrong frame")]
    WrongFrame(frame::Kind),
}

#[derive(Debug, Error)]
pub enum PrefaceErrorState {
    // server
    #[error("read preface")]
    ReadPreface,
    #[error("poll server settings")]
    PollServerSettings,
    #[error("read client settings")]
    ReadClientSettings,
    #[error("send client settings ack")]
    PollClientSettingsAck,

    // common
    #[error("flush")]
    Flush,

    // client
    #[error("write preface")]
    WritePreface,
    #[error("read server settings")]
    ReadServerSettings,
    #[error("send client settings")]
    SendClientSettings,
}

pub trait IoStateExt<T> {
    fn in_state(self, state: PrefaceErrorState) -> Result<T, PrefaceError>;
}

impl<T> IoStateExt<T> for Result<T, std::io::Error> {
    #[inline]
    fn in_state(self, state: PrefaceErrorState) -> Result<T, PrefaceError> {
        self.map_err(|e| PrefaceError::new(state, PrefaceErrorKind::from(e)))
    }
}

impl<T> IoStateExt<T> for Result<T, UserError> {
    #[inline]
    fn in_state(self, state: PrefaceErrorState) -> Result<T, PrefaceError> {
        self.map_err(|e| PrefaceError::new(state, PrefaceErrorKind::from(e)))
    }
}

// TODO: Implement display for proto
impl<T> IoStateExt<T> for Result<T, proto::Error> {
    #[inline]
    fn in_state(self, state: PrefaceErrorState) -> Result<T, PrefaceError> {
        self.map_err(|e| PrefaceError::new(state, PrefaceErrorKind::Proto))
    }
}
