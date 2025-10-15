#![allow(warnings, dead_code)]

macro_rules! proto_err {
    (conn: $($msg:tt)+) => {
        tracing::debug!("connection error PROTOCOL_ERROR -- {};", format_args!($($msg)+))
    };
    (stream: $($msg:tt)+) => {
        tracing::debug!("stream error PROTOCOL_ERROR -- {};", format_args!($($msg)+))
    };
}

macro_rules! ready {
    ($e:expr) => {
        match $e {
            ::std::task::Poll::Ready(r) => r,
            ::std::task::Poll::Pending => return ::std::task::Poll::Pending,
        }
    };
}

pub mod builder;
pub mod codec;
mod ext;
mod frame;
mod hpack;
pub mod io;
pub mod preface;
pub mod proto;
pub mod role;
pub mod state;

// tests
pub use crate::proto::connection::Connection;
pub use codec::Codec;
pub use frame::*;
