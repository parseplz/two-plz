use crate::builder::Role;
use crate::codec::{Codec, UserError};
use crate::frame::{Frame, Settings};
use crate::proto::Error as ProtoError;
use bytes::BytesMut;
use futures::StreamExt;
use futures::future::poll_fn;
use tokio::io::AsyncWriteExt;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};
use tracing::info;
mod error;
use error::*;

/*
 client MUST => preface
 client MUST => SETTINGS
                SETTINGS     <= server MUST
 client MAY  => Frames
 client MUST => SETTINGS ack
                SETTINGS ack <= server MUST
*/

const PREFACE: [u8; 24] = *b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";

pub struct PrefaceConn<T> {
    role: Role,
    stream: Codec<T, BytesMut>,
    local_settings: Settings,
    remote_settings: Option<Settings>,
}

impl<T> PrefaceConn<T>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    pub fn new(stream: T, role: Role) -> Self {
        PrefaceConn {
            role,
            stream: Codec::new(stream),
            local_settings: Settings::default(),
            remote_settings: None,
        }
    }

    pub fn get_mut(&mut self) -> &mut T {
        self.stream.get_mut()
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

    pub async fn read_frame(&mut self) -> Option<Result<Frame, ProtoError>> {
        self.stream.next().await
    }

    pub async fn flush(&mut self) -> Result<(), std::io::Error> {
        poll_fn(|cx| self.stream.flush(cx)).await
    }
}

pub enum ServerPreface<T> {
    PrefaceExchange(PrefaceConn<T>),
    PollLocalSettings(PrefaceConn<T>),
    ReadPeerSettings(PrefaceConn<T>),
    PollPeerSettingsAck(PrefaceConn<T>),
    Flush(PrefaceConn<T>),
    End(PrefaceConn<T>),
}

impl<T> ServerPreface<T>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    pub fn new(stream: T, role: Role) -> Self {
        ServerPreface::PrefaceExchange(PrefaceConn::new(stream, role))
    }

    pub async fn next(self) -> Result<ServerPreface<T>, PrefaceError> {
        let next_state = match self {
            ServerPreface::PrefaceExchange(mut conn) => {
                info!("[+] read client preface");
                match conn.role {
                    Role::Server => {
                        let mut buf = BytesMut::new();
                        loop {
                            let _ = conn
                                .get_mut()
                                .read_buf(&mut buf)
                                .await
                                .in_state(PrefaceErrorState::ReadPreface)?;
                            if buf.len() > 23 {
                                if buf.starts_with(&PREFACE) {
                                    buf.split_to(24);
                                    // add remaning buf to front of codec reader
                                    if !buf.is_empty() {
                                        conn.prepend_to_inner_buf(buf);
                                    }
                                    break;
                                } else {
                                    Err(PrefaceErrorKind::InvalidPreface)?
                                }
                            }
                        }
                    }
                    Role::Client => {
                        conn.get_mut()
                            .write(&PREFACE)
                            .await
                            .in_state(PrefaceErrorState::WritePreface)?;
                        conn.get_mut()
                            .flush()
                            .await
                            .in_state(PrefaceErrorState::Flush)?;
                    }
                }
                Self::PollLocalSettings(conn)
            }
            ServerPreface::PollLocalSettings(mut conn) => {
                info!("[+] send server settings to client");
                conn.poll_local_settings()
                    .in_state(PrefaceErrorState::PollServerSettings)?;
                Self::ReadPeerSettings(conn)
            }
            Self::ReadPeerSettings(mut conn) => {
                info!("[+] read client settings");
                let frame = conn
                    .read_frame()
                    .await
                    .ok_or(PrefaceError::new(
                        PrefaceErrorState::ReadClientSettings,
                        PrefaceErrorKind::Eof,
                    ))?
                    .in_state(PrefaceErrorState::ReadClientSettings)?;
                if let Frame::Settings(settings) = frame {
                    conn.remote_settings = Some(settings);
                    Self::PollPeerSettingsAck(conn)
                } else {
                    return Err(PrefaceError::new(
                        PrefaceErrorState::ReadClientSettings,
                        PrefaceErrorKind::WrongFrame(frame.kind()),
                    ));
                }
            }
            Self::PollPeerSettingsAck(mut conn) => {
                info!("[+] send settings ack to client");
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
