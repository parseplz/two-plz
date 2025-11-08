use crate::{
    Codec, Connection, Response, StreamId,
    builder::{BuildConnection, Builder},
    proto::{config::ConnectionConfig, streams::streams_ref::StreamRef},
    request::Request,
    role::Role,
};
use bytes::{Buf, BytesMut};
use futures::future::poll_fn;
use std::{
    io::Error,
    task::{Context, Poll},
};
use tokio::io::{AsyncRead, AsyncWrite};

// ===== Builder =====
pub struct Server;
pub type ServerBuilder = Builder<Server>;

impl BuildConnection for Server {
    type Connection<T, B> = ServerConnection<T, B>;

    fn is_server() -> bool {
        true
    }

    fn is_client() -> bool {
        false
    }

    fn init_stream_id() -> StreamId {
        2.into()
    }

    fn build<T, B>(
        role: Role,
        config: ConnectionConfig,
        codec: Codec<T, BytesMut>,
    ) -> Self::Connection<T, B>
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

pub struct ServerConnection<T, B> {
    connection: Connection<T, B>,
}

impl<T, B> ServerConnection<T, B>
where
    T: AsyncRead + AsyncWrite + Unpin,
    B: Buf,
{
    pub async fn accept(
        &mut self,
    ) -> Option<Result<(Request, SendResponse<B>), crate::Error>> {
        poll_fn(move |cx| self.poll_accept(cx)).await
    }

    pub fn poll_accept(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<(Request, SendResponse<B>), crate::Error>>> {
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
    fn send_response(&mut self, response: Response) -> Result<(), Error> {
        todo!()
    }
}
