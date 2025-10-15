use crate::StreamId;

#[derive(PartialEq, Clone)]
pub enum Role {
    Client,
    Server,
}

impl Role {
    pub fn is_server(&self) -> bool {
        matches!(self, Self::Server)
    }

    pub fn is_client(&self) -> bool {
        matches!(self, Self::Client)
    }

    pub fn init_stream_id(&self) -> StreamId {
        if self.is_server() {
            2.into()
        } else {
            1.into()
        }
    }
}
