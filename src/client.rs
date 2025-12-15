use std::{
    pin::Pin,
    task::{Context, Poll},
};

use bytes::Bytes;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::{
    Codec, Connection,
    builder::{BuildConnection, Builder},
    error::{OpError, Reason},
    frame::StreamId,
    message::{request::Request, response::Response},
    proto::{config::ConnectionConfig, streams::Streams},
    role::Role,
};

use crate::proto::streams::OpaqueStreamRef;

// ===== Builder =====
pub struct Client;

pub type ClientBuilder = Builder<Client>;

impl BuildConnection for Client {
    type Connection<T> = (ClientConnection<T>, SendRequest);

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
        let conn = ClientConnection {
            inner: Connection::new(role, config, codec),
        };
        let send_request = SendRequest {
            inner: conn.inner.streams.clone(),
        };
        (conn, send_request)
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

impl<T> Future for ClientConnection<T>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    type Output = Result<(), OpError>;

    fn poll(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Self::Output> {
        self.inner
            .maybe_close_connection_if_no_streams();
        let had_streams_or_refs = self
            .inner
            .has_streams_or_other_references();
        let result = self.inner.poll(cx).map_err(Into::into);
        // if we had streams/refs, and don't anymore, wake up one more time to
        // ensure proper shutdown

        //dbg!(result.is_pending());
        //dbg!(had_streams_or_refs);
        //dbg!(
        //    self.inner
        //        .has_streams_or_other_references()
        //);
        if result.is_pending()
            && had_streams_or_refs
            && !self
                .inner
                .has_streams_or_other_references()
        {
            cx.waker().wake_by_ref();
        }
        result
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

    pub fn num_active_streams(&self) -> usize {
        self.inner.num_active_streams()
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
    type Output = Result<Response, PartialResponse>;

    fn poll(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Self::Output> {
        self.inner
            .poll_response(cx)
            .map_err(Into::into)
    }
}
