#[macro_use]
pub mod assert;
pub mod frames;
pub mod mock;
pub mod prelude;
pub mod raw;
pub mod trace;
pub mod util;

mod client_ext;
mod future_ext;

use http::Method;
use two_plz::hpack::BytesStr;
use two_plz::message::request::uri::{Scheme, Uri};
use two_plz::message::request::{Request, RequestBuilder};

pub use crate::client_ext::SendRequestExt;
pub use crate::future_ext::TestFuture;

pub type WindowSize = usize;
pub const DEFAULT_WINDOW_SIZE: WindowSize = (1 << 16) - 1;

pub type Codec<T> = two_plz::Codec<T, bytes::Bytes>;
pub type SendFrame = two_plz::frame::Frame<bytes::Bytes>;

#[macro_export]
macro_rules! trace_init {
    () => {
        let _guard = $crate::trace::init();
        let span = $crate::prelude::tracing::info_span!(
            "test",
            "{}",
            // get the name of the test thread to generate a unique span for the test
            std::thread::current()
                .name()
                .expect("test threads must be named")
        );
        let _e = span.enter();
    };
}
