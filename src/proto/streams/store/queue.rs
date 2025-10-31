use std::{fmt, marker::PhantomData};

use crate::proto::streams::{
    store::{Indices, Key, Ptr, Resolve},
    stream::Stream,
};

pub struct Queue<N> {
    indices: Option<super::Indices>,
    _p: PhantomData<N>,
}

impl<N> Queue<N>
where
    N: Next,
{
    pub fn new() -> Self {
        Queue {
            indices: None,
            _p: PhantomData,
        }
    }

    pub fn take(&mut self) -> Self {
        Queue {
            indices: self.indices.take(),
            _p: PhantomData,
        }
    }

    /// Queue the stream.
    ///
    /// If the stream is already contained by the list, return `false`.
    pub fn push(&mut self, stream: &mut Ptr) -> bool {
        tracing::trace!("Queue::push_back");

        if N::is_queued(stream) {
            tracing::trace!(" -> already queued");
            return false;
        }

        N::set_queued(stream, true);

        // The next pointer shouldn't be set
        debug_assert!(N::next(stream).is_none());

        // Queue the stream
        match self.indices {
            Some(ref mut idxs) => {
                tracing::trace!(" -> existing entries");

                // Update the current tail node to point to `stream`
                let key = stream.key();
                N::set_next(&mut stream.resolve(idxs.tail), Some(key));

                // Update the tail pointer
                idxs.tail = stream.key();
            }
            None => {
                tracing::trace!(" -> first entry");
                self.indices = Some(Indices {
                    head: stream.key(),
                    tail: stream.key(),
                });
            }
        }

        true
    }

    /// Queue the stream
    ///
    /// If the stream is already contained by the list, return `false`.
    pub fn push_front(&mut self, stream: &mut Ptr) -> bool {
        tracing::trace!("Queue::push_front");

        if N::is_queued(stream) {
            tracing::trace!(" -> already queued");
            return false;
        }

        N::set_queued(stream, true);

        // The next pointer shouldn't be set
        debug_assert!(N::next(stream).is_none());

        // Queue the stream
        match self.indices {
            Some(ref mut idxs) => {
                tracing::trace!(" -> existing entries");

                // Update the provided stream to point to the head node
                let head_key = stream.resolve(idxs.head).key();
                N::set_next(stream, Some(head_key));

                // Update the head pointer
                idxs.head = stream.key();
            }
            None => {
                tracing::trace!(" -> first entry");
                self.indices = Some(Indices {
                    head: stream.key(),
                    tail: stream.key(),
                });
            }
        }

        true
    }

    pub fn pop<'a, R>(&mut self, store: &'a mut R) -> Option<Ptr<'a>>
    where
        R: Resolve,
    {
        if let Some(mut idxs) = self.indices {
            let mut stream = store.resolve(idxs.head);

            if idxs.head == idxs.tail {
                assert!(N::next(&stream).is_none());
                self.indices = None;
            } else {
                idxs.head = N::take_next(&mut stream).unwrap();
                self.indices = Some(idxs);
            }

            debug_assert!(N::is_queued(&stream));
            N::set_queued(&mut stream, false);

            return Some(stream);
        }

        None
    }

    pub fn is_empty(&self) -> bool {
        self.indices.is_none()
    }

    pub fn pop_if<'a, R, F>(
        &mut self,
        store: &'a mut R,
        f: F,
    ) -> Option<Ptr<'a>>
    where
        R: Resolve,
        F: Fn(&Stream) -> bool,
    {
        if let Some(idxs) = self.indices {
            let should_pop = f(&store.resolve(idxs.head));
            if should_pop {
                return self.pop(store);
            }
        }

        None
    }
}

impl<N> fmt::Debug for Queue<N> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("Queue")
            .field("indices", &self.indices)
            // skip phantom data
            .finish()
    }
}

pub trait Next {
    fn next(stream: &Stream) -> Option<Key>;

    fn set_next(stream: &mut Stream, key: Option<Key>);

    fn take_next(stream: &mut Stream) -> Option<Key>;

    fn is_queued(stream: &Stream) -> bool;

    fn set_queued(stream: &mut Stream, val: bool);
}
