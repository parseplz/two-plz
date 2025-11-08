use bytes::BytesMut;
use http::{HeaderMap, HeaderValue, StatusCode, Version};

use crate::response::Response;

#[derive(Default)]
pub struct ResponseBuilder {
    pub version: Version,
    pub status: StatusCode,
    pub headers: HeaderMap<HeaderValue>,
    pub body: Option<BytesMut>,
}

impl ResponseBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn version(mut self, v: Version) -> Self {
        self.version = v;
        self
    }

    pub fn status(mut self, s: StatusCode) -> Self {
        self.status = s;
        self
    }

    pub fn build(self) -> Response {
        Response {
            version: self.version,
            status: self.status,
            headers: self.headers,
            body: self.body,
        }
    }
}
