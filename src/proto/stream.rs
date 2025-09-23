use crate::{frame::StreamId, proto::buffer::Deque};

#[derive(Debug)]
pub struct Stream {
    pub(crate) id: StreamId,
    queue: Deque,
}
