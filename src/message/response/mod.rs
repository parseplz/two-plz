use http::{HeaderMap, StatusCode};

use crate::{
    frame::{StreamId, headers::Pseudo},
    message::{InfoLine, TwoTwo},
    proto::ProtoError,
};

mod builder;
pub use builder::ResponseBuilder;

pub type Response = TwoTwo<ResponseLine>;

impl Response {
    pub fn status(&self) -> StatusCode {
        self.info_line.status
    }
}

#[derive(Debug)]
pub struct ResponseLine {
    pub status: StatusCode,
}

impl InfoLine for ResponseLine {
    fn into_pseudo(self) -> Pseudo {
        Pseudo::response(self.status)
    }
}

impl Response {
    pub fn from_http_two(
        pseudo: Pseudo,
        headers: HeaderMap,
        _stream_id: StreamId,
    ) -> Result<Response, ProtoError> {
        let mut b = ResponseBuilder::new();
        if let Some(status) = pseudo.status {
            b = b.status(status);
        }
        b.headers = headers;
        Ok(b.build())
    }
}
