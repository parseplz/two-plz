use bytes::BytesMut;
use http::{HeaderMap, HeaderValue};
pub mod request;
pub mod response;


#[derive(Debug)]
pub struct TwoTwo<T> {
    info_line: T,
    headers: HeaderMap<HeaderValue>,
    body: Option<BytesMut>,
    trailer: Option<HeaderMap<HeaderValue>>,
}
