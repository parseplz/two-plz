use crate::codec::UserError;
use crate::proto::streams::store::{Resolve, Store};
use crate::{
    Codec, Connection, StreamId,
    builder::{BuildConnection, Builder},
    frame,
    headers::Pseudo,
    message::{
        request::Request,
        response::{Response, ResponseLine},
    },
    proto::{config::ConnectionConfig, streams::streams_ref::StreamRef},
    role::Role,
};
use bytes::{Buf, Bytes};
use futures::future::poll_fn;
use http::{HeaderMap, HeaderValue};
use std::{
    io::Error,
    task::{Context, Poll},
};
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
    ) -> Option<Result<(Request, SendResponse<Bytes>), crate::Error>> {
        poll_fn(move |cx| self.poll_accept(cx)).await
    }

    pub fn poll_accept(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<(Request, SendResponse<Bytes>), crate::Error>>>
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
}

#[derive(Debug)]
pub struct SendResponse<B: Buf> {
    inner: StreamRef<B>,
}

impl<B: Buf> SendResponse<B> {
    fn send_response(&mut self, response: Response) -> Result<(), UserError> {
        self.inner.send_response(response)
    }
}
