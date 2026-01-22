use super::prelude::*;
use http_plz::Request;
use two_plz::client::{ResponseFuture, SendRequest};

/// Extend the `h2::client::SendRequest` type with convenience methods.
pub trait SendRequestExt {
    /// Convenience method to send a GET request and ignore the SendStream
    /// (since GETs don't need to send a body).
    fn get(&mut self, uri: &str) -> ResponseFuture;
}

impl SendRequestExt for SendRequest {
    fn get(&mut self, path: &str) -> ResponseFuture {
        let mut b = Uri::builder();
        b = b.path(path);
        let request = Request::builder()
            .method(Method::GET)
            .uri(b.build().unwrap())
            .build();
        self.send_request(request)
            .expect("send_request")
    }
}
