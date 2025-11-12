use crate::Reason;
use bytes::BytesMut;
use http::{HeaderMap, HeaderValue, Method, StatusCode, Version};
pub mod request;
pub mod response;

use crate::{StreamId, ext::Protocol, headers::Pseudo, proto::ProtoError};

#[derive(Debug)]
pub struct TwoTwo<T> {
    info_line: T,
    headers: HeaderMap<HeaderValue>,
    body: Option<BytesMut>,
    trailer: Option<HeaderMap<HeaderValue>>,
}
