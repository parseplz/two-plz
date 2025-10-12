use tokio::io::{Empty, empty};
use two_plz::{
    Codec, Connection,
    proto::{
        config::ConnectionConfig,
        connection::{ClientToUser, ServerToUser, UserToClient, UserToServer},
    },
};

pub fn build_server() -> Connection<Empty, ServerToUser, UserToServer> {
    let (conn, _) =
        Connection::server(ConnectionConfig::default(), Codec::new(empty()));
    conn
}

pub fn build_client() -> Connection<Empty, ClientToUser, UserToClient> {
    let (conn, _) =
        Connection::client(ConnectionConfig::default(), Codec::new(empty()));
    conn
}

pub fn write_to_read_buf<T, E, U>(conn: &mut Connection<T, E, U>) {
    let write_buf = conn.stream.write_buf_mut().split();
    conn.stream
        .read_buf_mut()
        .unsplit(write_buf);
}
