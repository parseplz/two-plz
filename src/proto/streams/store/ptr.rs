use super::{Resolve, SlabIndex, Store, Stream};
use crate::frame::StreamId;
use std::fmt;
use std::fmt::Debug;
use std::ops::{Deref, DerefMut};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Key {
    pub(crate) index: SlabIndex,
    /// Keep the stream ID in the key as an ABA guard, since slab indices
    /// could be re-used with a new stream.
    pub(crate) stream_id: StreamId,
}

/// "Pointer" to an entry in the store
pub(crate) struct Ptr<'a> {
    pub(crate) key: Key,
    pub(crate) store: &'a mut Store,
}

impl<'a> Ptr<'a> {
    /// Returns the Key associated with the stream
    pub fn key(&self) -> Key {
        self.key
    }

    pub fn store_mut(&mut self) -> &mut Store {
        self.store
    }

    /// Remove the stream from the store
    pub fn remove(self) -> StreamId {
        // The stream must have been unlinked before this point
        debug_assert!(
            !self
                .store
                .ids
                .contains_key(&self.key.stream_id)
        );

        // Remove the stream state
        let stream = self
            .store
            .slab
            .remove(self.key.index.0 as usize);
        assert_eq!(stream.id, self.key.stream_id);
        stream.id
    }

    /// Remove the StreamId -> stream state association.
    ///
    /// This will effectively remove the stream as far as the H2 protocol is
    /// concerned.
    pub fn unlink(&mut self) {
        let id = self.key.stream_id;
        self.store.ids.swap_remove(&id);
    }
}

impl<'a> Resolve for Ptr<'a> {
    fn resolve(&mut self, key: Key) -> Ptr<'_> {
        Ptr {
            key,
            store: &mut *self.store,
        }
    }
}

impl<'a> Deref for Ptr<'a> {
    type Target = Stream;

    fn deref(&self) -> &Stream {
        &self.store[self.key]
    }
}

impl<'a> DerefMut for Ptr<'a> {
    fn deref_mut(&mut self) -> &mut Stream {
        &mut self.store[self.key]
    }
}

impl<'a> Debug for Ptr<'a> {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        (**self).fmt(fmt)
    }
}
