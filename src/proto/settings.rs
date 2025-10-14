use crate::{Reason, frame::Settings, proto};

pub enum SettingsAction {
    SendAck,
    ApplyLocal(Settings),
}

#[derive(Debug)]
pub(crate) struct SettingsHandler {
    /// Our local SETTINGS sync state with the remote.
    local: Local,
    /// Received SETTINGS frame pending processing. The ACK must be written to
    /// the socket first then the settings applied **before** receiving any
    /// further frames.
    remote: Option<Settings>,
    /// Whether the connection has received the initial SETTINGS frame from the
    /// remote peer.
    has_received_remote_initial_settings: bool,
}

#[derive(Debug)]
enum Local {
    /// We want to send these SETTINGS to the remote when the socket is ready.
    ToSend(Settings),
    /// We have sent these SETTINGS and are waiting for the remote to ACK
    /// before we apply them.
    WaitingAck(Settings),
    /// Our local settings are in sync with the remote.
    Synced,
}

impl SettingsHandler {
    pub(crate) fn new(local: Settings) -> Self {
        SettingsHandler {
            // We assume the initial local SETTINGS were flushed during
            // the handshake process.
            local: Local::WaitingAck(local),
            remote: None,
            has_received_remote_initial_settings: false,
        }
    }

    pub fn recv(
        &mut self,
        frame: Settings,
    ) -> Result<SettingsAction, proto::Error> {
        if frame.is_ack() {
            match &self.local {
                Local::WaitingAck(settings) => {
                    let ret = SettingsAction::ApplyLocal(settings.clone());
                    self.local = Local::Synced;
                    Ok(ret)
                }
                Local::ToSend(..) | Local::Synced => {
                    // We haven't sent any SETTINGS frames to be ACKed, so
                    // this is very bizarre! Remote is either buggy or malicious.
                    proto_err!(conn: "received unexpected settings ack");
                    Err(proto::Error::library_go_away(Reason::PROTOCOL_ERROR))
                }
            }
        } else {
            self.remote = Some(frame);
            Ok(SettingsAction::SendAck)
        }
    }

    pub fn take_remote_settings(&mut self) -> Settings {
        // safe to unwrap
        self.remote.take().unwrap()
    }

    pub fn add_pending_ack(&mut self, frame: Settings) {
        self.local = Local::WaitingAck(frame);
    }
}
