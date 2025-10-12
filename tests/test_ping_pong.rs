use futures::StreamExt;
use two_plz::{
    frame::{Frame, Ping},
    state::read::{ReadState, read_runner},
};
mod common;
use common::*;

#[test]
fn recv_single_ping() {
    let mut conn = build_server();
    let frame = Ping::new(Default::default());
    let state = read_runner(&mut conn, frame.into()).unwrap();
    assert_eq!(state, ReadState::NeedsFlush);
    write_to_read_buf(&mut conn);
    let frame = conn.read_frame().unwrap();
    let ping = assert_ping!(frame);
    assert_eq!(ping, Ping::pong(Default::default()));
}

#[tokio::test]
async fn recv_multiple_ping() {
    let mut conn = build_server();
    for i in 1..3 {
        let frame = Ping::new([i; 8]);
        let state = read_runner(&mut conn, frame.into()).unwrap();
        assert_eq!(state, ReadState::NeedsFlush);
    }
    write_to_read_buf(&mut conn);
    for i in 1..3 {
        let ping = poll_frame!(Ping, conn.stream);
        assert_eq!(ping, Ping::pong([i; 8]));
    }
}

// TODO
//async fn user_notifies_when_connection_closes() {
