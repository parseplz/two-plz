use crate::frame::Reason;
use crate::{frame::Reset, proto::store::Store};

struct RstHandler;

impl RstHandler {
    fn recv(frame: Reset, store: &mut Store) -> Result<(), Reason> {
        if let Some(mut ptr) = store.find_mut(&frame.stream_id()) {
            ptr.unlink();
            ptr.remove();
            Ok(())
        } else {
            Err(Reason::STREAM_CLOSED)
        }
    }
}
