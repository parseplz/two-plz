#![allow(warnings, dead_code)]
use tokio::io::{Empty, empty};
use two_plz::{
    Codec, Connection, Settings,
    proto::{
        config::ConnectionConfig,
        connection::{ClientToUser, ServerToUser, UserToClient, UserToServer},
    },
};

//
#[macro_export]
macro_rules! poll_frame {
    ($type: ident, $transport:expr) => {{
        use futures::StreamExt;

        match $transport.next().await {
            Some(Ok(Frame::$type(frame))) => frame,
            frame => panic!("unexpected frame; actual={:?}", frame),
        }
    }};
}

#[macro_export]
macro_rules! assert_ping {
    ($frame:expr) => {{
        match $frame {
            Frame::Ping(v) => v,
            f => panic!("expected PING; actual={:?}", f),
        }
    }};
}

pub fn build_server() -> Connection<Empty, ServerToUser, UserToServer> {
    let (conn, _) = Connection::server(
        ConnectionConfig::default(),
        Codec::new(empty()),
        Settings::default(),
    );
    conn
}

pub fn build_client() -> Connection<Empty, ClientToUser, UserToClient> {
    let (conn, _) = Connection::client(
        ConnectionConfig::default(),
        Codec::new(empty()),
        Settings::default(),
    );
    conn
}

pub fn write_to_read_buf<T, E, U>(conn: &mut Connection<T, E, U>) {
    let write_buf = conn.stream.write_buf_mut().split();
    conn.stream
        .read_buf_mut()
        .unsplit(write_buf);
}
