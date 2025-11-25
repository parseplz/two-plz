use bytes::BytesMut;
use http::{HeaderMap, HeaderValue};

use crate::frame::{self, StreamId, headers::Pseudo};
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

impl<T> TwoTwo<T> {
    pub fn new(
        info_line: T,
        headers: HeaderMap<HeaderValue>,
        body: Option<BytesMut>,
        trailer: Option<HeaderMap<HeaderValue>>,
    ) -> TwoTwo<T> {
        Self {
            info_line,
            headers,
            body,
            trailer,
        }
    }
}

pub struct TwoTwoFrame {
    pub(crate) header: frame::Headers,
    data: Option<frame::Data>,
    trailer: Option<frame::Headers>,
}

impl TwoTwoFrame {
    pub fn take_data(&mut self) -> Option<frame::Data> {
        self.data.take()
    }

    pub fn take_trailer(&mut self) -> Option<frame::Headers> {
        self.trailer.take()
    }
}

impl<T> From<(StreamId, TwoTwo<T>)> for TwoTwoFrame
where
    T: InfoLine,
{
    fn from((stream_id, mut message): (StreamId, TwoTwo<T>)) -> Self {
        let (body, trailer) = (message.body, message.trailer);
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

        let data = body.map(|b| {
            let mut frame = frame::Data::new(stream_id, b.freeze());
            if trailer.is_none() {
                frame.set_end_stream(true);
            }
            frame
        });

        let trailer = trailer.map(|t| frame::Headers::trailers(stream_id, t));

        TwoTwoFrame {
            header,
            data,
            trailer,
        }
    }
}
