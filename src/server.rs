use std::io::Error;

use bytes::BytesMut;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::{
    Codec, Connection, Request, Response, StreamId,
    builder::{BuildConnection, Builder},
    proto::config::ConnectionConfig,
    role::Role,
};

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
        codec: Codec<T, BytesMut>,
    ) -> Self::Connection<T>
    where
        T: AsyncRead + AsyncWrite + Unpin,
    {
        ServerConnection {
            conn: Connection::new(role, config, codec),
        }
    }
}

// server handshake => ServerConnection
// ServerConnection.accept() => Request, SendResponse
// Request => complete request
// SendResponse.send_response(Response)

pub struct ServerConnection<T> {
    conn: Connection<T>,
}

impl<T> ServerConnection<T> {
    async fn accept(&mut self) -> Result<(Request, SendResponse), Error> {
        todo!()
    }
}

struct SendResponse;

impl SendResponse {
    fn send_response(&mut self, response: Response) -> Result<(), Error> {
        todo!()
    }
}
