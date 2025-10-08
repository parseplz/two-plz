use std::marker::PhantomData;

use crate::proto::{store::Key, stream::Stream};

pub struct Queue<N> {
    indices: Option<super::Indices>,
    _p: PhantomData<N>,
}

pub trait Next {
    fn next(stream: &Stream) -> Option<Key>;

    fn set_next(stream: &mut Stream, key: Option<Key>);

    fn take_next(stream: &mut Stream) -> Option<Key>;

    fn is_queued(stream: &Stream) -> bool;

    fn set_queued(stream: &mut Stream, val: bool);
}
