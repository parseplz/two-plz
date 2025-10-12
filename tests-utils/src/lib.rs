#![allow(dead_code, unused)]
use tokio::io::{Empty, empty};
use two_plz::{
    Codec, Connection,
    preface::Role,
    proto::{
        config::{ConnectionConfig, PeerRole},
        connection::{ServerToUser, UserToServer},
    },
};

pub fn build_server() -> Connection<Empty, ServerToUser, UserToServer> {
    Connection::server(ConnectionConfig::default(), Codec::new(empty()))
}

pub fn build_client() -> Connection<Empty, UserToServer, ServerToUser> {
    Connection::client(ConnectionConfig::default(), Codec::new(empty()))
}
