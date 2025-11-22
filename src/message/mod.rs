use bytes::BytesMut;
use http::{HeaderMap, HeaderValue};

use crate::{StreamId, frame, headers::Pseudo};
pub mod request;
pub mod response;

pub trait InfoLine {
    fn into_pseudo(self) -> Pseudo;
}

#[derive(Debug)]
pub struct TwoTwo<T> {
    info_line: T,
    headers: HeaderMap<HeaderValue>,
    body: Option<BytesMut>,
    trailer: Option<HeaderMap<HeaderValue>>,
}

pub struct TwoTwoFrame {
    header: frame::Headers,
    data: Option<frame::Data>,
    trailer: Option<frame::Headers>,
}

impl<T> From<(StreamId, TwoTwo<T>)> for TwoTwoFrame
where
    T: InfoLine,
{
    pub fn take_body(&mut self) -> Option<BytesMut> {
        self.body.take()
    }

    pub fn take_trailer(&mut self) -> Option<HeaderMap<HeaderValue>> {
        self.trailer.take()
    }

    pub fn into_message_head(self) -> (T, HeaderMap<HeaderValue>) {
        (self.info_line, self.headers)
    }

    pub fn into_header_frame(self, stream_id: StreamId) -> frame::Headers {
        let pseudo = self.info_line.into_pseudo();
        frame::Headers::new(stream_id, pseudo, self.headers)
    }
}

pub trait InfoLine {
    fn into_pseudo(self) -> Pseudo;
}
