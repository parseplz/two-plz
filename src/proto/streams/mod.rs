mod action;
mod buffer;
mod counts;
mod flow_control;
pub mod opaque_streams_ref;
mod recv;
pub mod send;
pub mod send_buffer;
pub mod streams_ref;
use send::Send;
mod inner;
mod state;
pub mod store;
mod stream;
use crate::proto::streams::{
    counts::Counts,
    recv::Recv,
    store::{Ptr, Resolve},
};
use store::Store;
pub mod streams;
use self::stream::Stream;

use crate::proto::ProtoError;
pub(crate) use recv::Open;
