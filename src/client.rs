use bytes::Bytes;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::{
    Codec, Connection,
    builder::{BuildConnection, Builder},
    error::OpError,
    frame::{self, StreamId},
    message::{request::Request, response::Response},
    proto::{
        config::ConnectionConfig,
        streams::{opaque_streams_ref::OpaqueStreamRef, streams::Streams},
    },
    role::Role,
};

// ===== Builder =====
pub struct Client;

pub type ClientBuilder = Builder<Client>;

impl BuildConnection for Client {
    type Connection<T> = ClientConnection<T>;

    fn is_server() -> bool {
        false
    }

    fn is_client() -> bool {
        true
    }

    fn init_stream_id() -> StreamId {
        1.into()
    }

    fn build<T>(
        role: Role,
        config: ConnectionConfig,
        codec: Codec<T, Bytes>,
    ) -> Self::Connection<T>
    where
        T: AsyncRead + AsyncWrite + Unpin,
    {
        ClientConnection {
            inner: Connection::new(role, config, codec),
        }
    }
}

// client handshake => ClientConnection , SendRequest
// spawn || ClientConnection
// SendRequest.send_request(Request) => RecvResponse
// RecvResponse.recv_response().await => Response
// Response => compelete response

pub struct ClientConnection<T> {
    inner: Connection<T>,
}

impl<T> ClientConnection<T> {
    pub fn send_request_handle(&self) -> SendRequest {
        SendRequest {
            inner: self.inner.streams.clone(),
        }
    }
}


// ===== Send Request ====
#[derive(Clone)]
pub struct SendRequest {
    inner: Streams<Bytes>,
}

impl SendRequest {
    pub fn send_request(
        &mut self,
        request: Request,
    ) -> Result<ResponseFuture, OpError> {
        self.inner
            .send_request(request)
            .map_err(Into::into)
            .map(|s| ResponseFuture {
                inner: s,
            })
    }
}

// ===== Response Future =====
pub struct ResponseFuture {
    inner: OpaqueStreamRef,
}

impl ResponseFuture {
    pub fn stream_id(&self) -> StreamId {
        self.inner.stream_id()
    }
}

impl Future for ResponseFuture {
    type Output = Result<Response, OpError>;

    fn poll(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Self::Output> {
        self.inner
            .poll_response(cx)
            .map_err(Into::into)
    }
}
