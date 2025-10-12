#![allow(warnings, dead_code)]

use tokio::io::{Empty, empty};
use two_plz::{
    Codec, Connection,
    preface::Role,
    proto::{
        config::{ConnectionConfig, PeerRole},
        connection::{ServerToUser, UserToServer},
    },
};
mod mock_server;
mod ping_pong;
extern crate two_plz;

fn build_server() -> Connection<Empty, ServerToUser, UserToServer> {
    Connection::server(ConnectionConfig::default(), Codec::new(empty()))
}

fn build_client() -> Connection<Empty, UserToServer, ServerToUser> {
    Connection::client(ConnectionConfig::default(), Codec::new(empty()))
}
