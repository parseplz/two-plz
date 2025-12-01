use http::Method;
use two_plz::{
    client::{ResponseFuture, SendRequest},
    hpack::BytesStr,
    message::request::{RequestBuilder, uri::Uri},
};

/// Extend the `h2::client::SendRequest` type with convenience methods.
pub trait SendRequestExt {
    /// Convenience method to send a GET request and ignore the SendStream
    /// (since GETs don't need to send a body).
    fn get(&mut self, uri: &str) -> ResponseFuture;
}

impl SendRequestExt for SendRequest {
    fn get(&mut self, path: &str) -> ResponseFuture {
        let mut uri = Uri::new(None, None, None);
        uri = uri.path(BytesStr::from(path));
        let request = RequestBuilder::new()
            .method(Method::GET)
            .uri(uri)
            .build();
        self.send_request(request)
            .expect("send_request")
    }
}
