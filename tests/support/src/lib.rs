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
use prelude::*;

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

pub fn build_test_request() -> Request {
    Request::builder()
        .method(Method::GET)
        .uri(build_test_uri())
        .build()
}

pub fn build_test_uri() -> Uri {
    let mut b = Uri::builder();
    b = b.authority(BytesStr::from_static("http2.akamai.com"));
    b = b.scheme(Scheme::HTTPS);
    b.build().unwrap()
}

pub fn build_test_request_post(host: &str) -> Request {
    let mut b = Uri::builder();
    b = b.authority(BytesStr::unchecked_from_slice(host.as_bytes()));
    b = b.scheme(Scheme::HTTPS);
    Request::builder()
        .method(Method::POST)
        .uri(b.build().unwrap())
        .body("hello".into())
        .build()
}

pub fn build_test_response() -> Response {
    let scode = ResponseLine::new(StatusCode::from_u16(200).unwrap());
    let headers = HeaderMap::new();
    let body = None;
    let trailers = None;
    Response::new(scode, headers, body, trailers)
}
