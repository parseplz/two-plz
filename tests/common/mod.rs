#![allow(warnings, dead_code)]
use tokio::io::{Empty, empty};
use two_plz::{
    Codec, Connection, Settings,
    proto::{
        config::ConnectionConfig,
        connection::{ClientToUser, ServerToUser, UserToClient, UserToServer},
    },
    role::Role,
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

fn default_connection_config() -> ConnectionConfig {
    ConnectionConfig {
        initial_connection_window_size: Some(100),
        local_max_error_reset_streams: Some(100),
        reset_stream_duration: std::time::Duration::from_millis(5000),
        local_reset_stream_max: 100,
        local_settings: Settings::default(),
        peer_settings: Settings::default(),
        remote_reset_stream_max: 10,
    }
}

pub fn build_server_conn() -> Connection<Empty, ServerToUser, UserToServer> {
    let (conn, _) = Connection::new(
        Role::Server,
        default_connection_config(),
        Codec::new(empty()),
    );
    conn
}

pub fn build_client_conn() -> Connection<Empty, ClientToUser, UserToClient> {
    let (conn, _) = Connection::new(
        Role::Client,
        default_connection_config(),
        Codec::new(empty()),
    );
    conn
}

pub fn write_to_read_buf<T, E, U>(conn: &mut Connection<T, E, U>) {
    let write_buf = conn.stream.write_buf_mut().split();
    conn.stream
        .read_buf_mut()
        .unsplit(write_buf);
}
