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

impl TwoTwoFrame {
    fn take_data(&mut self) -> Option<frame::Data> {
        self.data.take()
    }

    fn take_trailer(&mut self) -> Option<frame::Headers> {
        self.trailer.take()
    }
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
}
