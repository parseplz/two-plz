use bytes::BytesMut;
use http::{HeaderMap, HeaderValue};

use crate::{StreamId, frame, headers::Pseudo};
pub mod request;
pub mod response;

#[derive(Debug)]
pub struct TwoTwo<T> {
    info_line: T,
    headers: HeaderMap<HeaderValue>,
    body: Option<BytesMut>,
    trailer: Option<HeaderMap<HeaderValue>>,
}

impl<T> TwoTwo<T>
where
    T: InfoLine,
{
    fn take_body(&mut self) -> Option<BytesMut> {
        self.body.take()
    }

    fn take_trailer(&mut self) -> Option<HeaderMap<HeaderValue>> {
        self.trailer.take()
    }

    pub fn into_message_head(self) -> (T, HeaderMap<HeaderValue>) {
        (self.info_line, self.headers)
    }

    pub fn into_header_frame(
        self,
        stream_id: StreamId,
        end_of_stream: bool,
    ) -> frame::Headers {
        let pseudo = self.info_line.into_pseudo();
        let mut frame = frame::Headers::new(stream_id, pseudo, self.headers);
        if end_of_stream {
            frame.set_end_stream()
        }
        frame
    }
}

