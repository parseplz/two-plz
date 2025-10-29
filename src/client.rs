use bytes::BytesMut;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::{
    Codec, Connection, Error, Request, Response, StreamId,
    builder::{BuildConnection, Builder},
    proto::config::ConnectionConfig,
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
        codec: Codec<T, BytesMut>,
    ) -> Self::Connection<T>
    where
        T: AsyncRead + AsyncWrite + Unpin,
    {
        ClientConnection {
            conn: Connection::new(role, config, codec),
        }
    }
}

// client handshake => ClientConnection , SendRequest
// spawn || ClientConnection
// SendRequest.send_request(Request) => RecvResponse
// RecvResponse.recv_response().await => Response
// Response => compelete response

pub struct ClientConnection<T> {
    pub conn: Connection<T>,
}

struct SendRequest;

impl SendRequest {
    fn send_request(
        &mut self,
        request: Request,
    ) -> Result<RecvResponse, Error> {
        todo!()
    }
}

struct RecvResponse;

impl RecvResponse {
    async fn recv_response(&mut self) -> Result<Response, Error> {
        todo!()
    }
}
