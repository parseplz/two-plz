use crate::codec::UserError;
use crate::error::{OpError, Reason};
use crate::{
    Codec, Connection,
    builder::{BuildConnection, Builder},
    frame::StreamId,
    message::{request::Request, response::Response},
    proto::{config::ConnectionConfig, streams::StreamRef},
    role::Role,
};
use bytes::Bytes;
use futures::future::poll_fn;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite};

// ===== Builder =====
pub struct Server;

pub type ServerBuilder = Builder<Server>;

impl BuildConnection for Server {
    type Connection<T> = ServerConnection<T>;

    fn is_server() -> bool {
        true
    }

    fn is_client() -> bool {
        false
    }

    fn init_stream_id() -> StreamId {
        2.into()
    }

    fn build<T>(
        role: Role,
        config: ConnectionConfig,
        codec: Codec<T, Bytes>,
    ) -> Self::Connection<T>
    where
        T: AsyncRead + AsyncWrite + Unpin,
    {
        ServerConnection {
            connection: Connection::new(role, config, codec),
        }
    }
}

// server handshake => ServerConnection
// ServerConnection.accept() => Request, SendResponse
// Request => complete request
// SendResponse.send_response(Response)

pub struct ServerConnection<T> {
    connection: Connection<T>,
}

impl<T> ServerConnection<T>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    pub async fn accept(
        &mut self,
    ) -> Option<Result<(Request, SendResponse), crate::frame::Error>> {
        poll_fn(move |cx| self.poll_accept(cx)).await
    }

    pub fn poll_accept(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<(Request, SendResponse), crate::frame::Error>>>
    {
        //TODO .map_err(Into::into);
        if self.connection.poll(cx).is_ready() {
            // If the socket is closed, don't return anything
            // TODO: drop any pending streams
            return Poll::Ready(None);
        }

        if let Some(inner) = self.connection.next_accept() {
            tracing::trace!("received incoming");
            let request = inner.take_request();
            let respond = SendResponse {
                inner,
            };
            return Poll::Ready(Some(Ok((request, respond))));
        }
        Poll::Pending
    }

    pub fn poll_closed(
        &mut self,
        cx: &mut Context,
    ) -> Poll<Result<(), OpError>> {
        self.connection
            .poll(cx)
            .map_err(Into::into)
    }

    pub fn num_wired_streams(&self) -> usize {
        self.connection.num_wired_streams()
    }
}

#[cfg(feature = "stream")]
impl<T> futures_core::Stream for ServerConnection<T>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    type Item = Result<(Request, SendResponse), crate::frame::Error>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        self.poll_accept(cx)
    }
}

#[derive(Debug)]
pub struct SendResponse {
    inner: StreamRef,
}

impl SendResponse {
    pub fn send_response(
        &mut self,
        response: Response,
    ) -> Result<(), UserError> {
        self.inner.send_response(response)
    }

    pub fn send_reset(&mut self, reason: Reason) {
        self.inner.send_reset(reason)
    }
}
