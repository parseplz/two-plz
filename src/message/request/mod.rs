use crate::Reason;
use crate::StreamId;
use crate::codec::UserError;
use crate::ext::Protocol;
use crate::headers::Pseudo;
use crate::message::InfoLine;
use crate::message::TwoTwo;
use crate::proto::ProtoError;
use bytes::BytesMut;
use http::uri::Scheme;
use http::{HeaderMap, Method, StatusCode};
mod builder;
pub mod uri;
use builder::RequestBuilder;
use uri::Uri;

pub type Request = TwoTwo<RequestLine>;

/*
         foo://example.com:8042/over/there?name=ferret#nose
         \_/   \______________/\_________/ \_________/ \__/
          |           |            |            |        |
       scheme     authority       path        query   fragment
          |   _____________________|__
         / \ /                        \
         urn:example:animal:ferret:nose

scheme      => anything
authority   => - if present neglect host header
               - convert from h11 omitted in origin-form and asterisk form
               - origin-form    = absolute-path [ "?" query ]
               - must be same as HOST header
path        => not empty for http/s - if empty "/"
               - except OPTIONS - "*" must
               - CONNECT - omitted

method , scheme , path => must

Connect request:
    method - connect
    scheme and path - omitted
    authority - host:port
*/

#[derive(Debug)]
pub struct RequestLine {
    method: Method,
    uri: Uri,
    extension: Option<Protocol>,
}

impl InfoLine for RequestLine {
    fn into_pseudo(self) -> Pseudo {
        // TODO
        // let is_connect = self.method == Method::CONNECT;
        let mut pseudo =
            Pseudo::request(self.method, self.uri, self.extension);

        if pseudo.scheme.is_none() {
            pseudo.set_scheme(Scheme::HTTP)
        }

        pseudo
    }
}

impl Request {
    pub fn from_http_two(
        pseudo: Pseudo,
        headers: HeaderMap,
        stream_id: StreamId,
    ) -> Result<Request, ProtoError> {
        // macro to return error
        macro_rules! malformed {
            ($($arg:tt)*) => {{
                tracing::debug!($($arg)*);
                return Err(ProtoError::library_reset(stream_id, Reason::PROTOCOL_ERROR));
            }}
        }

        // check status code in request
        if pseudo.status.is_some() {
            malformed!("malformed headers| :status field on request");
        }

        let mut b = RequestBuilder::new();

        // method check
        let is_connect;
        if let Some(method) = pseudo.method {
            is_connect = method == Method::CONNECT;
            b = b.method(method);
        } else {
            malformed!("malformed headers| missing method");
        }

        // add protocol for CONNECT requests
        let has_protocol = pseudo.protocol.is_some();
        if has_protocol {
            if is_connect {
                b = b.extension(pseudo.protocol.unwrap());
            } else {
                malformed!(
                    "malformed headers| :protocol on non-CONNECT request"
                );
            }
        }

        /// Uri
        let mut uri = Uri::default();

        // authority
        if let Some(authority) = pseudo.authority {
            uri = uri.authority(authority);
        }

        // A :scheme is required, except CONNECT.
        if let Some(scheme) = pseudo.scheme {
            if is_connect && !has_protocol {
                malformed!("malformed headers| :scheme in CONNECT");
            }
            let maybe_scheme = scheme.parse();
            let scheme = maybe_scheme.or_else(|why| {
                malformed!(
                    "malformed headers| malformed scheme ({:?}): {}",
                    scheme,
                    why,
                )
            })?;

            // It's not possible to build an `Uri` from a scheme and path. So,
            // after validating is was a valid scheme, we just have to drop it
            // if there isn't an :authority.
            if uri.authority.is_some() {
                uri = uri.scheme(scheme);
            }
        } else if !is_connect || has_protocol {
            malformed!("malformed headers| missing scheme");
        }

        // path
        if let Some(path) = pseudo.path {
            if is_connect && !has_protocol {
                malformed!("malformed headers| :path in CONNECT");
            }

            // This cannot be empty
            if path.is_empty() {
                malformed!("malformed headers| missing path");
            }
            uri = uri.path(path);
        } else if is_connect && has_protocol {
            malformed!("malformed headers| missing path in extended CONNECT");
        }

        b.headers = headers;
        b = b.uri(uri);

        Ok(b.build())
    }

    pub fn set_trailer(&mut self, trailer: HeaderMap) {
        self.trailer = Some(trailer);
    }

    pub fn set_body(&mut self, body: Option<BytesMut>) {
        self.body = body;
    }
}
