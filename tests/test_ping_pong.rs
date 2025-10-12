use two_plz::{
    frame::{Frame, Ping},
    state::read::{ReadState, read_runner},
};
mod common;
use common::*;

#[test]
fn test_ping_pong() {
    let mut conn = build_server();
    let frame = Ping::new(Default::default());
    let state = read_runner(&mut conn, frame.into()).unwrap();
    assert_eq!(state, ReadState::NeedsFlush);
    write_to_read_buf(&mut conn);
    let frame = conn.read_frame().unwrap();
    if let Frame::Ping(ping) = frame {
        assert_eq!(ping, Ping::pong(Default::default()));
    } else {
        panic!()
    }

    //
}
