pub mod connection;

mod buffer;
pub mod config;
mod count;
mod error;
mod flow_control;
pub mod ping_pong;
pub mod recv;
mod rst_stream;
pub mod send;
pub mod settings;
mod state;
mod store;
mod stream;
mod streams;

pub use error::ProtoError;

/////////////////////
pub type PingPayload = [u8; 8];

pub type WindowSize = u32;

// Constants
pub const MAX_WINDOW_SIZE: WindowSize = (1 << 31) - 1; // i32::MAX as u32

// Max no of streams that can live after reset
pub const DEFAULT_REMOTE_RESET_COUNT_MAX: usize = 20;
pub const DEFAULT_LOCAL_RESET_COUNT_MAX: usize = 1024;

// RFC 9113 suggests allowing at minimum 100 streams, it seems reasonable to
// by default allow a portion of that to be remembered as reset for some time.
pub const DEFAULT_RESET_STREAM_MAX: usize = 50;

// RFC 9113#5.4.2 suggests ~1 RTT. We don't track that closely, but use a
// reasonable guess of the average here.
pub const DEFAULT_RESET_STREAM_SECS: u64 = 1;
