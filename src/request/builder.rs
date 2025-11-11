use super::*;
use crate::request::uri::UriBuilder;
use http::{HeaderMap, HeaderValue, Method, Version};

#[derive(Default)]
pub struct RequestBuilder {
    pub method: Method,
    pub uri: UriBuilder,
    pub version: Version,
    pub headers: HeaderMap<HeaderValue>,
    pub extension: Option<Protocol>,
    pub body: Option<BytesMut>,
    pub trailer: Option<HeaderMap<HeaderValue>>,
}

impl RequestBuilder {
    pub fn new() -> Self {
        RequestBuilder::default()
    }

    pub fn method(mut self, m: Method) -> Self {
        self.method = m;
        self
    }

    pub fn version(mut self, v: Version) -> Self {
        self.version = v;
        self
    }

    pub fn extension(mut self, p: Protocol) -> Self {
        self.extension = Some(p);
        self
    }

    pub fn uri(mut self, u: UriBuilder) -> Self {
        self.uri = u;
        self
    }

    pub fn build(self) -> Request {
        Request {
            method: self.method,
            uri: self.uri.build(),
            version: self.version,
            headers: self.headers,
            extension: self.extension,
            body: self.body,
            trailer: self.trailer,
        }
    }
}
