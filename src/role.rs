use http::HeaderMap;

use crate::{
    Reason, Request, Response, StreamId,
    headers::Pseudo,
    proto::{ProtoError, recv::Open},
};

#[derive(Debug)]
pub enum PollMessage {
    Client(Response),
    Server(Request),
}

#[derive(PartialEq, Clone, Debug)]
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

    pub fn is_local_init(&self, id: StreamId) -> bool {
        assert!(!id.is_zero());
        self.is_server() == id.is_server_initiated()
    }

    /// Returns true if the remote peer can initiate a stream with the given ID.
    pub fn ensure_can_open(
        &self,
        id: StreamId,
        mode: Open,
    ) -> Result<(), ProtoError> {
        if self.is_server() {
            // Ensure that the ID is a valid client initiated ID
            if mode.is_push_promise() || !id.is_client_initiated() {
                proto_err!(conn: "cannot open stream {:?} - not client initiated", id);
                return Err(ProtoError::library_go_away(
                    Reason::PROTOCOL_ERROR,
                ));
            }

            Ok(())
        } else {
            // Ensure that the ID is a valid server initiated ID
            if !mode.is_push_promise() || !id.is_server_initiated() {
                proto_err!(conn: "cannot open stream {:?} - not server initiated", id);
                return Err(ProtoError::library_go_away(
                    Reason::PROTOCOL_ERROR,
                ));
            }

            Ok(())
        }
    }

    pub fn convert_poll_message(
        &self,
        pseudo: Pseudo,
        fields: HeaderMap,
        stream_id: StreamId,
    ) -> Result<PollMessage, ProtoError> {
        todo!()
    }
}
