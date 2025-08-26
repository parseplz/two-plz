use crate::frame::{Head, StreamId, error::Error};

/*
PRIORITY Frame {
    Length (24) = 0x05,
    Type (8) = 0x02,

    Unused Flags (8),

    Reserved (1),
    Stream Identifier (31),

    Exclusive (1),
    Stream Dependency (31),
    Weight (8),
}
*/

#[derive(Debug, Eq, PartialEq)]
pub struct StreamDependency {
    /// The ID of the stream dependency target
    dependency_id: StreamId,

    /// The weight for the stream. The value exposed (and set) here is always in
    /// the range [0, 255], instead of [1, 256] (as defined in section 5.3.2.)
    /// so that the value fits into a `u8`.
    weight: u8,

    /// True if the stream dependency is exclusive.
    is_exclusive: bool,
}

impl StreamDependency {
    pub fn new(
        dependency_id: StreamId,
        weight: u8,
        is_exclusive: bool,
    ) -> Self {
        StreamDependency {
            dependency_id,
            weight,
            is_exclusive,
        }
    }

    pub fn load(src: &[u8]) -> Result<Self, Error> {
        if src.len() != 5 {
            return Err(Error::InvalidPayloadLength);
        }

        // Parse the stream ID and exclusive flag
        let (dependency_id, is_exclusive) = StreamId::parse(&src[..4]);

        // Read the weight
        let weight = src[4];

        Ok(StreamDependency::new(dependency_id, weight, is_exclusive))
    }

    pub fn dependency_id(&self) -> StreamId {
        self.dependency_id
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct Priority {
    stream_id: StreamId,
    dependency: StreamDependency,
}

impl Priority {
    pub fn load(head: Head, payload: &[u8]) -> Result<Self, Error> {
        let dependency = StreamDependency::load(payload)?;

        if dependency.dependency_id() == head.stream_id() {
            return Err(Error::InvalidDependencyId);
        }

        Ok(Priority {
            stream_id: head.stream_id(),
            dependency,
        })
    }
}

//impl<B> From<Priority> for Frame<B> {
//    fn from(src: Priority) -> Self {
//        Frame::Priority(src)
//    }
//}
