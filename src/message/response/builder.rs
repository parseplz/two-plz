use bytes::BytesMut;
use http::{HeaderMap, HeaderValue, StatusCode, Version};

use crate::message::response::{Response, ResponseLine};

#[derive(Default)]
pub struct ResponseBuilder {
    pub status: StatusCode,
    pub headers: HeaderMap<HeaderValue>,
    pub body: Option<BytesMut>,
}

impl ResponseBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn status(mut self, s: StatusCode) -> Self {
        self.status = s;
        self
    }

    pub fn build(self) -> Response {
        Response {
            info_line: ResponseLine {
                status: self.status,
            },
            headers: self.headers,
            body: self.body,
            trailer: None,
        }
    }
}
