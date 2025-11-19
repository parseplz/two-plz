use http::{HeaderMap, StatusCode};

use crate::{
    StreamId,
    headers::Pseudo,
    message::{InfoLine, TwoTwo},
    proto::ProtoError,
};

mod builder;
use builder::ResponseBuilder;

pub type Response = TwoTwo<ResponseLine>;

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
        stream_id: StreamId,
    ) -> Result<Response, ProtoError> {
        let mut b = ResponseBuilder::new();
        if let Some(status) = pseudo.status {
            b = b.status(status);
        }
        b.headers = headers;
        Ok(b.build())
    }
}
