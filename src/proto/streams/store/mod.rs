use std::{
    convert::Infallible,
    ops::{Index, IndexMut},
};

use indexmap::{self, IndexMap};

use crate::frame::StreamId;
use crate::proto::streams::stream::Stream;
mod entry;
mod ptr;
mod queue;
pub(crate) use entry::*;
pub(crate) use ptr::{Key, Ptr};
pub(crate) use queue::{Next, Queue};

pub trait Resolve {
    fn resolve(&mut self, key: Key) -> Ptr<'_>;
}

// We can never have more than `StreamId::MAX` streams in the store,
// so we can save a smaller index (u32 vs usize).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SlabIndex(u32);

#[derive(Debug)]
pub struct Store {
    slab: slab::Slab<Stream>,
    ids: IndexMap<StreamId, SlabIndex>,
}

impl Store {
    pub fn new() -> Self {
        Store {
            slab: slab::Slab::new(),
            ids: IndexMap::new(),
        }
    }

    pub fn insert(&mut self, id: StreamId, val: Stream) -> Ptr<'_> {
        let index = SlabIndex(self.slab.insert(val) as u32);
        assert!(self.ids.insert(id, index).is_none());

        Ptr {
            key: Key {
                index,
                stream_id: id,
            },
            store: self,
        }
    }

    pub fn find_entry(&mut self, id: StreamId) -> Entry<'_> {
        use self::indexmap::map::Entry::*;

        match self.ids.entry(id) {
            Occupied(e) => Entry::Occupied(OccupiedEntry {
                ids: e,
            }),
            Vacant(e) => Entry::Vacant(VacantEntry {
                ids: e,
                slab: &mut self.slab,
            }),
        }
    }

    pub fn find_mut(&mut self, id: &StreamId) -> Option<Ptr<'_>> {
        let index = match self.ids.get(id) {
            Some(key) => *key,
            None => return None,
        };

        Some(Ptr {
            key: Key {
                index,
                stream_id: *id,
            },
            store: self,
        })
    }

    #[allow(clippy::blocks_in_conditions)]
    pub(crate) fn for_each<F>(&mut self, mut f: F)
    where
        F: FnMut(Ptr),
    {
        match self.try_for_each(|ptr| {
            f(ptr);
            Ok::<_, Infallible>(())
        }) {
            Ok(()) => (),
            #[allow(unused)]
            Err(infallible) => match infallible {},
        }
    }

    pub fn try_for_each<F, E>(&mut self, mut f: F) -> Result<(), E>
    where
        F: FnMut(Ptr) -> Result<(), E>,
    {
        let mut len = self.ids.len();
        let mut i = 0;

        while i < len {
            // Get the key by index, this makes the borrow checker happy
            let (stream_id, index) = {
                let entry = self.ids.get_index(i).unwrap();
                (*entry.0, *entry.1)
            };

            f(Ptr {
                key: Key {
                    index,
                    stream_id,
                },
                store: self,
            })?;

            // TODO: This logic probably could be better...
            let new_len = self.ids.len();

            if new_len < len {
                debug_assert!(new_len == len - 1);
                len -= 1;
            } else {
                i += 1;
            }
        }

        Ok(())
    }
}

impl Index<Key> for Store {
    type Output = Stream;

    fn index(&self, key: Key) -> &Self::Output {
        self.slab
            .get(key.index.0 as usize)
            .filter(|s| s.id == key.stream_id)
            .unwrap_or_else(|| {
                panic!("dangling store key for stream_id={:?}", key.stream_id);
            })
    }
}

impl IndexMut<Key> for Store {
    fn index_mut(&mut self, key: Key) -> &mut Self::Output {
        self.slab
            .get_mut(key.index.0 as usize)
            .filter(|s| s.id == key.stream_id)
            .unwrap_or_else(|| {
                panic!("dangling store key for stream_id={:?}", key.stream_id);
            })
    }
}

impl Resolve for Store {
    fn resolve(&mut self, key: Key) -> Ptr<'_> {
        Ptr {
            key,
            store: self,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Indices {
    pub head: Key,
    pub tail: Key,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_store_insert() {
        let store = Store::new();
    }
}
