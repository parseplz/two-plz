use crate::{Reason, message::TwoTwo};
use bytes::BytesMut;
use http::{HeaderMap, Method, StatusCode};
mod builder;
mod uri;
use builder::RequestBuilder;
use uri::Uri;
use uri::UriBuilder;

use crate::{StreamId, ext::Protocol, headers::Pseudo, proto::ProtoError};

pub type Request = TwoTwo<RequestLine>;

#[derive(Debug)]
pub struct RequestLine {
    method: Method,
    uri: Uri,
    extension: Option<Protocol>,
}

#[derive(Debug)]
struct ResponseLine {
    status: StatusCode,
}

impl Request {
    pub fn from_http_two(
        pseudo: Pseudo,
        headers: HeaderMap,
        stream_id: StreamId,
        body: Option<BytesMut>,
    ) -> Result<Request, ProtoError> {
        let mut b = RequestBuilder::new();

        // macro to return error
        macro_rules! malformed {
            ($($arg:tt)*) => {{
                tracing::debug!($($arg)*);
                return Err(ProtoError::library_reset(stream_id, Reason::PROTOCOL_ERROR));
            }}
        }

        // connect method check
        let is_connect;
        if let Some(method) = pseudo.method {
            is_connect = method == Method::CONNECT;
            b = b.method(method);
        } else {
            malformed!("malformed headers: missing method");
        }

        // add protocol extension
        let has_protocol = pseudo.protocol.is_some();
        if has_protocol {
            if is_connect {
                b = b.extension(pseudo.protocol.unwrap());
            } else {
                malformed!(
                    "malformed headers: :protocol on non-CONNECT request"
                );
            }
        }

        // check status code in request
        if pseudo.status.is_some() {
            malformed!("malformed headers: :status field on request");
        }

        /// Uri
        let mut uri = UriBuilder::default();

        // authority
        if let Some(authority) = pseudo.authority {
            uri = uri.authority(authority);
        }

        // A :scheme is required, except CONNECT.
        if let Some(scheme) = pseudo.scheme {
            if is_connect && !has_protocol {
                malformed!("malformed headers: :scheme in CONNECT");
            }
            let maybe_scheme = scheme.parse();
            let scheme = maybe_scheme.or_else(|why| {
                malformed!(
                    "malformed headers: malformed scheme ({:?}): {}",
                    scheme,
                    why,
                )
            })?;

            // It's not possible to build an `Uri` from a scheme and path. So,
            // after validating is was a valid scheme, we just have to drop it
            // if there isn't an :authority.
            if uri.authority.is_some() {
                uri.scheme = Some(scheme);
            }
        } else if !is_connect || has_protocol {
            malformed!("malformed headers: missing scheme");
        }

        // path
        if let Some(path) = pseudo.path {
            if is_connect && !has_protocol {
                malformed!("malformed headers: :path in CONNECT");
            }

            // This cannot be empty
            if path.is_empty() {
                malformed!("malformed headers: missing path");
            }
            uri = uri.path(path);
        } else if is_connect && has_protocol {
            malformed!("malformed headers: missing path in extended CONNECT");
        }

        b.headers = headers;
        b = b.uri(uri);
        b.body = body;

        Ok(b.build())
    }

    pub fn set_trailer(&mut self, trailer: HeaderMap) {
        self.trailer = Some(trailer);
    }

    pub fn set_body(&mut self, body: Option<BytesMut>) {
        self.body = body;
    }
}
