use crate::frame::Settings;

pub struct SettingsHandler {
    local: Settings,
    peer: Settings,

    /// local settings to be ack by peer
    /// Some means we have to wait for ack
    local_to_ack: Option<Settings>,

    /// if true, frame is waiting to send
    /// after send, set to false
    awaiting_local_send: bool,

    /// peer settings to be ack by local
    /// Some means we have to send settings ack
    peer_to_ack: Option<Settings>,

    /// preface settings acknowledged by peer
    initial_acked: bool,
}

/*
We cannot expect the peer to acknowledge our settings during the preface state.
The peer may send other frames before acking our settings.
We have already ack the peer settings during preface state.
so, we expect local settings to be ack by peer and peer settings is already acked
*/

enum SettingsAction {
    ApplyLocal,
    SendAck,
    Unknown,
    InitialAck,
}

impl SettingsHandler {
    pub fn new(local_to_ack: Settings, peer: Settings) -> Self {
        Self {
            local: Settings::default(),
            peer,
            awaiting_local_send: false,
            local_to_ack: Some(local_to_ack),
            peer_to_ack: None,
            initial_acked: false,
        }
    }

    // read
    pub fn recv_settings(&mut self, frame: Settings) -> SettingsAction {
        if frame.is_ack() {
            match self.local_to_ack.take() {
                Some(local_to_ack) => {
                    if self.initial_acked {
                        self.local = local_to_ack;
                        SettingsAction::ApplyLocal
                    } else {
                        self.initial_acked = true;
                        SettingsAction::InitialAck
                    }
                }
                None => SettingsAction::Unknown,
            }
        } else {
            self.peer = frame;
            self.peer_to_ack = Some(Settings::ack());
            SettingsAction::SendAck
        }
    }

    // write
    pub fn send(&mut self, frame: Settings) {
        self.local_to_ack = Some(frame);
        self.awaiting_local_send = true;
    }

    pub fn local(&self) -> &Settings {
        &self.local
    }

    pub fn peer(&self) -> &Settings {
        &self.peer
    }
}
