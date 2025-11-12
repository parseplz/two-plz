use std::task::{Context, Poll};

use bytes::{Buf, BytesMut};
use tokio::io::AsyncWrite;

use crate::{
    Codec, Reason,
    codec::UserError,
    frame::Settings,
    proto::{self, ProtoError, streams::streams::Streams},
};

pub enum SettingsAction {
    /// send a SETTINGS ACK for remote SETTINGS
    Ok,
    /// SETTINGS ACK received from peer apply local settings
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
        }
    }

    pub fn recv(
        &mut self,
        frame: Settings,
    ) -> Result<SettingsAction, proto::ProtoError> {
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
                    Err(proto::ProtoError::library_go_away(
                        Reason::PROTOCOL_ERROR,
                    ))
                }
            }
        } else {
            self.remote = Some(frame);
            Ok(SettingsAction::Ok)
        }
    }

    pub fn poll_remote_settings<T, B, C>(
        &mut self,
        cx: &mut Context,
        dst: &mut Codec<T, B>,
        streams: &mut Streams<C>,
    ) -> Poll<Result<(), ProtoError>>
    where
        T: AsyncWrite + Unpin,
        B: Buf,
        C: Buf,
    {
        if let Some(settings) = self.remote.clone() {
            if !dst.poll_ready(cx)?.is_ready() {
                return Poll::Pending;
            }
            // Create an ACK settings frame
            let frame = Settings::ack();
            // Buffer the settings frame
            dst.buffer(frame.into())
                .expect("invalid settings frame");
            streams.apply_remote_settings(&settings)?;

            if let Some(val) = settings.header_table_size() {
                dst.set_send_header_table_size(val as usize);
            }

            if let Some(val) = settings.max_frame_size() {
                dst.set_max_send_frame_size(val as usize);
            }
        }
        self.remote = None;
        Poll::Ready(Ok(()))
    }

    pub fn add_pending_ack(&mut self, frame: Settings) {
        self.local = Local::WaitingAck(frame);
    }
}
