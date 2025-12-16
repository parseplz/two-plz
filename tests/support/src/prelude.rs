pub use two_plz;
use two_plz::client::ClientConnection;
pub use two_plz::*;

// Re-export Request and Response build functions
pub use super::build_test_request;
pub use super::build_test_request_post;
pub use super::build_test_response;

// Re-export mock
pub use super::mock::{self, idle_ms};

// Re-export frames helpers
pub use super::frames;

// Re-export utility mod
pub use super::util;

// Re-export some type defines
pub use super::{Codec, SendFrame};

// Re-export macros
pub use super::{
    assert_closed, assert_data, assert_default_settings, assert_go_away,
    assert_headers, assert_ping, assert_settings, poll_err, poll_frame,
    raw_codec,
};

pub use super::assert::assert_frame_eq;

// Re-export useful crates
pub use tokio_test::io as mock_io;
pub use {
    bytes, futures, http, tokio::io as tokio_io, tracing, tracing_subscriber,
};

// Re-export two-plz
pub use two_plz::client;
pub use two_plz::client::ClientBuilder;
pub use two_plz::ext::Protocol;
pub use two_plz::frame::StreamId;
pub use two_plz::message::request::Request;
pub use two_plz::message::response::Response;
pub use two_plz::message::response::ResponseBuilder;
pub use two_plz::message::response::ResponseLine;
pub use two_plz::server;
pub use two_plz::server::ServerBuilder;

// Re-export primary future types
pub use futures::{Future, Sink, Stream};

// And our Future extensions
pub use super::future_ext::{
    TestFuture, join, join_all, join3, join4, poll_once, select, try_join,
};

// Our client_ext helpers
pub use super::client_ext::SendRequestExt;

// Re-export HTTP types
pub use http::{HeaderMap, Method, StatusCode, Version, uri};

pub use bytes::{Buf, BufMut, Bytes, BytesMut};

pub use tokio::io::{AsyncRead, AsyncWrite};

pub use std::thread;
pub use std::time::Duration;

pub use frame::Reason;

pub static MAGIC_PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";

// ===== Everything under here shouldn't be used =====
// TODO: work on deleting this code

use futures::future;
use futures::future::Either::*;
pub use futures::future::poll_fn;
pub use std::pin::Pin;
pub use std::task::Poll;

pub trait MockH2 {
    fn handshake(&mut self) -> &mut Self;

    fn handshake_read_settings(&mut self, settings: &[u8]) -> &mut Self;
}

impl MockH2 for tokio_test::io::Builder {
    fn handshake(&mut self) -> &mut Self {
        self.handshake_read_settings(frames::SETTINGS)
    }

    fn handshake_read_settings(&mut self, settings: &[u8]) -> &mut Self {
        self.write(MAGIC_PREFACE)
            .write(frames::NEW_SETTINGS)
            .read(settings)
            .write(frames::SETTINGS_ACK)
    }
}

pub trait ClientExt {
    fn run<'a, F: Future + Unpin + 'a>(
        &'a mut self,
        f: F,
    ) -> Pin<Box<dyn Future<Output = F::Output> + 'a>>;
}

impl<T> ClientExt for ClientConnection<T>
where
    T: AsyncRead + AsyncWrite + Unpin + 'static,
{
    fn run<'a, F: Future + Unpin + 'a>(
        &'a mut self,
        f: F,
    ) -> Pin<Box<dyn Future<Output = F::Output> + 'a>> {
        let res = future::select(self, f);
        Box::pin(async {
            match res.await {
                Left((Ok(_), b)) => {
                    // Connection is done...
                    b.await
                }
                Right((v, _)) => v,
                Left((Err(e), _)) => panic!("err: {:?}", e),
            }
        })
    }
}

pub fn build_large_headers() -> Vec<(&'static str, String)> {
    vec![
        ("one", "hello".to_string()),
        ("two", build_large_string('2', 4 * 1024)),
        ("three", "three".to_string()),
        ("four", build_large_string('4', 4 * 1024)),
        ("five", "five".to_string()),
        ("six", build_large_string('6', 4 * 1024)),
        ("seven", "seven".to_string()),
        ("eight", build_large_string('8', 4 * 1024)),
        ("nine", "nine".to_string()),
        ("ten", build_large_string('0', 4 * 1024)),
        ("eleven", build_large_string('1', 32 * 1024)),
    ]
}

fn build_large_string(ch: char, len: usize) -> String {
    let mut ret = String::new();

    for _ in 0..len {
        ret.push(ch);
    }

    ret
}
