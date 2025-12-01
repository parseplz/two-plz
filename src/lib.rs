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
pub mod client;
pub mod codec;
pub mod error;
pub mod ext;
pub mod frame;
pub mod hpack;
pub mod io;
pub mod message;
pub mod preface;
pub mod proto;
pub mod role;
pub mod server;

// tests
pub use crate::proto::connection::Connection;
pub use codec::Codec;
