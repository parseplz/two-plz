use indexmap::{self};

use super::*;

pub enum Entry<'a> {
    Occupied(OccupiedEntry<'a>),
    Vacant(VacantEntry<'a>),
}

pub struct OccupiedEntry<'a> {
    pub ids: indexmap::map::OccupiedEntry<'a, StreamId, SlabIndex>,
}

pub struct VacantEntry<'a> {
    pub ids: indexmap::map::VacantEntry<'a, StreamId, SlabIndex>,
    pub slab: &'a mut slab::Slab<Stream>,
}

// ===== impl OccupiedEntry =====

impl<'a> OccupiedEntry<'a> {
    pub fn key(&self) -> Key {
        let stream_id = *self.ids.key();
        let index = *self.ids.get();
        Key {
            index,
            stream_id,
        }
    }
}

// ===== impl VacantEntry =====

impl<'a> VacantEntry<'a> {
    pub fn insert(self, value: Stream) -> Key {
        // Insert the value in the slab
        let stream_id = value.id;
        let index = SlabIndex(self.slab.insert(value) as u32);

        // Insert the handle in the ID map
        self.ids.insert(index);

        Key {
            index,
            stream_id,
        }
    }
}
