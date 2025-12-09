use crate::codec::{Codec, UserError};
use crate::frame::{Frame, Settings};
use crate::role::Role;
use bytes::{Bytes, BytesMut};
use futures::StreamExt;
use futures::future::poll_fn;
use tokio::io::AsyncWriteExt;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};
use tracing::info;
mod error;
pub use error::*;

/*
 client MUST => preface
 client MUST => SETTINGS
                SETTINGS     <= server MUST
 client MAY  => Frames
 client MUST => SETTINGS ack
                SETTINGS ack <= server MUST

client state
    1. Send Preface (24 bytes)
    2. Send SETTINGS
    3. Read first frame → MUST be server SETTINGS
    4. Send SETTINGS ACK
    5. Start sending/receiving requests

server state
    1. Send SETTINGS
    2. Flush SETTINGS to socket
    3. Read Preface (24 bytes)
    4. Read first frame → MUST be client SETTINGS
    5. Send SETTINGS ACK
    6. Start receiving/responding to requests
*/

const PREFACE: [u8; 24] = *b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";

pub struct PrefaceConn<T> {
    pub stream: Codec<T, Bytes>,
    pub role: Role,
    local_settings: Settings,
    remote_settings: Option<Settings>,
}

impl<T> PrefaceConn<T>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    pub fn new(stream: T, role: Role, local_settings: Settings) -> Self {
        PrefaceConn {
            role,
            stream: Codec::new(stream),
            local_settings,
            remote_settings: None,
        }
    }

    pub fn prepend_to_inner_buf(&mut self, buf: BytesMut) {
        let buf_mut = self.stream.read_buf_mut();
        let to_append = buf_mut.split();
        buf_mut.unsplit(buf);
        buf_mut.unsplit(to_append);
    }

    pub fn poll_local_settings(&mut self) -> Result<(), UserError> {
        self.stream
            .buffer(self.local_settings.clone().into())
    }

    pub fn poll_settings_ack(&mut self) -> Result<(), UserError> {
        self.stream
            .buffer(Settings::ack().into())
    }

    pub async fn flush(&mut self) -> Result<(), std::io::Error> {
        poll_fn(|cx| self.stream.flush(cx)).await
    }

    pub async fn read_preface(&mut self) -> Result<(), PrefaceError> {
        assert!(self.role.is_server());
        let mut buf = BytesMut::new();
        loop {
            let _ = self
                .stream
                .get_mut()
                .read_buf(&mut buf)
                .await
                .in_state(PrefaceErrorState::ReadPreface)?;
            if buf.len() > 23 {
                if buf.starts_with(&PREFACE) {
                    let _ = buf.split_to(24);
                    // add remaning buf to front of codec reader
                    if !buf.is_empty() {
                        self.prepend_to_inner_buf(buf);
                    }
                    break Ok(());
                } else {
                    Err(PrefaceErrorKind::InvalidPreface)?
                }
            }
        }
    }

    pub async fn read_frame(&mut self) -> Result<Frame, PrefaceError> {
        self.stream
            .next()
            .await
            .ok_or(PrefaceError::new(
                PrefaceErrorState::ReadClientSettings,
                PrefaceErrorKind::Eof,
            ))?
            .in_state(PrefaceErrorState::ReadClientSettings)
    }

    pub fn take_remote_settings(&mut self) -> Settings {
        // safe to unwrap
        self.remote_settings.take().unwrap()
    }
}

pub enum PrefaceState<T> {
    SendPreface(T, Settings),    // client only
    ReadPreface(PrefaceConn<T>), // server only
    SendLocalSettings(PrefaceConn<T>),
    ReadPeerSettings(PrefaceConn<T>),
    SendPeerSettingsAck(PrefaceConn<T>),
    Flush(PrefaceConn<T>),
    End(PrefaceConn<T>),
}

impl<T> PrefaceState<T>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    pub fn new(stream: T, role: Role, local_settings: Settings) -> Self {
        match role {
            Role::Client => Self::SendPreface(stream, local_settings),
            Role::Server => Self::SendLocalSettings(PrefaceConn::new(
                stream,
                role,
                local_settings,
            )),
        }
    }

    pub async fn next(self) -> Result<PrefaceState<T>, PrefaceError> {
        let next_state = match self {
            PrefaceState::SendPreface(mut stream, local_settings) => {
                info!("[+] send preface");
                stream
                    .write_all(&PREFACE)
                    .await
                    .in_state(PrefaceErrorState::WritePreface)?;
                let conn =
                    PrefaceConn::new(stream, Role::Client, local_settings);
                Self::SendLocalSettings(conn)
            }
            PrefaceState::SendLocalSettings(mut conn) => {
                info!("[+] send local settings to peer");
                conn.poll_local_settings()
                    .in_state(PrefaceErrorState::PollServerSettings)?;
                // TODO: can remove in favor of final flush ?
                conn.flush()
                    .await
                    .in_state(PrefaceErrorState::Flush)?;
                if conn.role == Role::Server {
                    Self::ReadPreface(conn)
                } else {
                    Self::ReadPeerSettings(conn)
                }
            }
            PrefaceState::ReadPreface(mut conn) => {
                info!("[+] read preface");
                conn.read_preface().await?;
                Self::ReadPeerSettings(conn)
            }
            Self::ReadPeerSettings(mut conn) => {
                info!("[+] read peer settings");
                let frame = conn.read_frame().await?;
                if let Frame::Settings(settings) = frame {
                    conn.remote_settings = Some(settings);
                    Self::SendPeerSettingsAck(conn)
                } else {
                    return Err(PrefaceError::new(
                        PrefaceErrorState::ReadClientSettings,
                        PrefaceErrorKind::WrongFrame(frame.kind()),
                    ));
                }
            }
            Self::SendPeerSettingsAck(mut conn) => {
                // apply peer settings
                let settings = conn.remote_settings.as_ref().unwrap();
                info!("[+] applying peer settings| {:?}", settings);
                if let Some(val) = settings.header_table_size() {
                    conn.stream
                        .set_send_header_table_size(val as usize);
                }

                if let Some(val) = settings.max_frame_size() {
                    conn.stream
                        .set_max_send_frame_size(val as usize);
                }
                info!("[+] send peer settings ack");
                conn.poll_settings_ack()
                    .in_state(PrefaceErrorState::PollClientSettingsAck)?;
                Self::Flush(conn)
            }
            Self::Flush(mut conn) => {
                info!("[+] flush");
                conn.flush()
                    .await
                    .in_state(PrefaceErrorState::Flush)?;
                Self::End(conn)
            }
            Self::End(_) => self,
        };
        Ok(next_state)
    }

    pub fn is_ended(&self) -> bool {
        matches!(self, Self::End(_))
    }
}

impl<T> TryFrom<PrefaceState<T>> for PrefaceConn<T> {
    type Error = PrefaceError;

    fn try_from(value: PrefaceState<T>) -> Result<Self, Self::Error> {
        if let PrefaceState::End(conn) = value {
            Ok(conn)
        } else {
            Err(PrefaceError::new(
                PrefaceErrorState::WrongState,
                PrefaceErrorKind::WrongState,
            ))
        }
    }
}
