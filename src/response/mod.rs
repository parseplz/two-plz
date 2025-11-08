use bytes::BytesMut;
use http::{HeaderMap, HeaderValue, StatusCode, Version};

use crate::{
    StreamId, headers::Pseudo, proto::ProtoError,
    response::builder::ResponseBuilder,
};

mod builder;

#[derive(Debug)]
pub struct Response {
    version: Version,
    status: StatusCode,
    headers: HeaderMap<HeaderValue>,
    pub body: Option<BytesMut>,
}

impl Response {
    pub fn from_http_two(
        pseudo: Pseudo,
        headers: HeaderMap,
        stream_id: StreamId,
    ) -> Result<Response, ProtoError> {
        let mut b = ResponseBuilder::new();
        b = b.version(Version::HTTP_2);
        if let Some(status) = pseudo.status {
            b = b.status(status);
        }
        b.headers = headers;
        Ok(b.build())
    }
}
