use bytes::{Buf, BytesMut};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::{
    Codec, Connection, Error, StreamId,
    builder::{BuildConnection, Builder},
    proto::config::ConnectionConfig,
    request::Request,
    response::Response,
    role::Role,
};

// ===== Builder =====
pub struct Client;
pub type ClientBuilder = Builder<Client>;

impl BuildConnection for Client {
    type Connection<T, B>
        = ClientConnection<T, B>
    where
        T: AsyncRead + AsyncWrite + Unpin,
        B: Buf;

    fn is_server() -> bool {
        false
    }

    fn is_client() -> bool {
        true
    }

    fn init_stream_id() -> StreamId {
        1.into()
    }

    fn build<T, B>(
        role: Role,
        config: ConnectionConfig,
        codec: Codec<T, B>,
    ) -> Self::Connection<T, B>
    where
        T: AsyncRead + AsyncWrite + Unpin,
        B: Buf,
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

pub struct ClientConnection<T, B: Buf> {
    pub conn: Connection<T, B>,
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
