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
    fn from((stream_id, mut message): (StreamId, TwoTwo<T>)) -> Self {
        let body = message.body;
        let trailer = message.trailer;
        let pseudo = message.info_line.into_pseudo();
        let mut header =
            frame::Headers::new(stream_id, pseudo, message.headers);
        if body.is_none() && trailer.is_none() {
            header.set_end_stream();
            return TwoTwoFrame {
                header,
                data: None,
                trailer: None,
            };
        }

        let data = body.map(|b| frame::Data::new(stream_id, b.freeze()));
        let trailer = trailer.map(|t| frame::Headers::trailers(stream_id, t));

        TwoTwoFrame {
            header,
            data,
            trailer,
        }
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
