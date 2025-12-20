use thiserror::Error;

use crate::{codec::UserError, frame, proto::ProtoError};

#[derive(Debug, Error)]
pub enum PrefaceError {
    #[error("invalid HTTP/2 preface received")]
    InvalidPreface,

    #[error("I/O error during {context}: {source}")]
    Io {
        context: &'static str,
        #[source]
        source: std::io::Error,
    },

    #[error("codec error during {context}: {source}")]
    Codec {
        context: &'static str,
        #[source]
        source: UserError,
    },

    #[error("protocol error during {context}: {source}")]
    Proto {
        context: &'static str,
        #[source]
        source: ProtoError,
    },

    #[error("unexpected EOF during read settings")]
    Eof,

    #[error("unexpected| {0:?}")]
    WrongFrame(frame::Kind),
}

pub trait ContextExt<T> {
    fn ctx(self, context: &'static str) -> Result<T, PrefaceError>;
}

impl<T> ContextExt<T> for Result<T, std::io::Error> {
    fn ctx(self, context: &'static str) -> Result<T, PrefaceError> {
        self.map_err(|source| PrefaceError::Io {
            context,
            source,
        })
    }
}

impl<T> ContextExt<T> for Result<T, UserError> {
    fn ctx(self, context: &'static str) -> Result<T, PrefaceError> {
        self.map_err(|source| PrefaceError::Codec {
            context,
            source,
        })
    }
}

impl<T> ContextExt<T> for Result<T, ProtoError> {
    fn ctx(self, context: &'static str) -> Result<T, PrefaceError> {
        self.map_err(|source| PrefaceError::Proto {
            context,
            source,
        })
    }
}
