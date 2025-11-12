use super::*;
use http::{HeaderMap, HeaderValue, Method};

#[derive(Default)]
pub struct RequestBuilder {
    pub method: Method,
    pub uri: UriBuilder,
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

    pub fn extension(mut self, p: Protocol) -> Self {
        self.extension = Some(p);
        self
    }

    pub fn uri(mut self, u: UriBuilder) -> Self {
        self.uri = u;
        self
    }

    pub fn build(self) -> Request {
        let info_line = RequestLine {
            method: self.method,
            uri: self.uri.build(),
            extension: self.extension,
        };
        Request {
            info_line,
            headers: self.headers,
            body: self.body,
            trailer: self.trailer,
        }
    }
}
