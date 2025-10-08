use crate::codec::{Codec, UserError};
use crate::frame::{self, Frame, HEADER_LEN, Head, Kind, Settings};
use crate::proto::Error as ProtoError;
use bytes::{BufMut, Bytes, BytesMut};
use futures::StreamExt;
use futures::future::poll_fn;
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, ReadHalf, WriteHalf};
use tokio_util::codec::{FramedRead as TokioFramedRead, LengthDelimitedCodec};
use tracing::info;
mod error;
use error::*;

use crate::io::{read_frame, write_and_flush};

const PREFACE: [u8; 24] = *b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";

pub struct PrefaceConn<T> {
    pub stream: Codec<T, BytesMut>,
    local_settings: Settings,
    remote_settings: Option<Settings>,
}

impl<T> PrefaceConn<T>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    pub fn new(stream: T) -> Self {
        PrefaceConn {
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
    ReadPreface(PrefaceConn<T>),
    PollServerSettings(PrefaceConn<T>),
    ReadClientSettings(PrefaceConn<T>),
    PollClientSettingsAck(PrefaceConn<T>),
    Flush(PrefaceConn<T>),
    End(PrefaceConn<T>),
}

impl<T> ServerPreface<T>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    pub fn new(stream: T) -> Self {
        ServerPreface::ReadPreface(PrefaceConn::new(stream))
    }

    pub async fn next(self) -> Result<ServerPreface<T>, PrefaceError> {
        let next_state = match self {
            ServerPreface::ReadPreface(mut conn) => {
                info!("[+] read client preface");
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
                            break Self::PollServerSettings(conn);
                        } else {
                            Err(PrefaceErrorKind::InvalidPreface)?
                        }
                    }
                }
            }
            ServerPreface::PollServerSettings(mut conn) => {
                info!("[+] send server settings to client");
                conn.poll_local_settings()
                    .in_state(PrefaceErrorState::PollServerSettings)?;
                Self::ReadClientSettings(conn)
            }
            Self::ReadClientSettings(mut conn) => {
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
                    Self::PollClientSettingsAck(conn)
                } else {
                    return Err(PrefaceError::new(
                        PrefaceErrorState::ReadClientSettings,
                        PrefaceErrorKind::WrongFrame(frame.kind()),
                    ));
                }
            }
            Self::PollClientSettingsAck(mut conn) => {
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

///////////////////////////

pub enum ClientPreface<T> {
    SendPreface(Codec<T, BytesMut>),
    SendClientSettings(Codec<T, BytesMut>),
    ReadServerSettings,
}

impl<T> ClientPreface<T>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    pub fn new(client: T) -> Self {
        ClientPreface::SendPreface(Codec::new(client))
    }

    pub async fn next(self) -> Result<ClientPreface<T>, PrefaceError> {
        let next_state = match self {
            ClientPreface::SendPreface(mut stream) => {
                info!("[+] read client preface");
                todo!()
            }
            _ => todo!(),
        };
        Ok(next_state)
    }
}

//////////////////////////

pub struct Preface<T, E> {
    client: T,
    server: E,
    buf: BytesMut,
}

impl<T, E> Preface<T, E> {
    fn new(client: T, server: E) -> Preface<T, E> {
        Preface {
            client,
            server,
            buf: BytesMut::new(),
        }
    }
}

type RawFrameReader<T> = TokioFramedRead<T, LengthDelimitedCodec>;

pub struct PrefaceFramed<T, E> {
    pub client_framed_reader: RawFrameReader<ReadHalf<T>>,
    pub client_writer: WriteHalf<T>,
    pub client_settings: Option<Settings>,
    pub is_client_settings_ack: bool,
    pub client_frame: Option<BytesMut>,
    pub server_framed_reader: RawFrameReader<ReadHalf<E>>,
    pub server_writer: WriteHalf<E>,
    pub server_settings: Option<Settings>,
    pub is_server_settings_ack: bool,
}

impl<T, E> From<Preface<T, E>> for PrefaceFramed<T, E>
where
    T: AsyncRead + AsyncWrite + Unpin,
    E: AsyncRead + AsyncWrite + Unpin,
{
    fn from(preface: Preface<T, E>) -> Self {
        todo!()
    }
}

/*
/*

 client MUST => preface
 client MUST => SETTINGS
                SETTINGS     <= server MUST
 client MAY  => Frames
 client MUST => SETTINGS ack
                SETTINGS ack <= server MUST
*/

pub enum HandshakeState<T, E>
where
    T: AsyncRead + AsyncWrite + Unpin,
    E: AsyncRead + AsyncWrite + Unpin,
{
    ReadPreface(Preface<T, E>),
    SendPreface(Preface<T, E>),
    // server settings => client
    ReadServerSettings(Preface<T, E>),
    SendServerSettings(PrefaceFramed<T, E>, BytesMut),
    // client settings => server
    ReadClientSettings(PrefaceFramed<T, E>),
    SendClientSettings(PrefaceFramed<T, E>, BytesMut),
    // End
    End(PrefaceFramed<T, E>),
}

impl<T, E> HandshakeState<T, E>
where
    T: AsyncRead + AsyncWrite + Unpin,
    E: AsyncRead + AsyncWrite + Unpin,
{
    pub fn init(client: T, server: E) -> HandshakeState<T, E> {
        HandshakeState::ReadPreface(Preface::new(client, server))
    }

    pub async fn next(self) -> Result<Self, PrefaceError> {
        let next_state = match self {
            HandshakeState::ReadPreface(mut conn) => {
                info!("[+] read client preface");
                loop {
                    let _ = conn
                        .client
                        .read_buf(&mut conn.buf)
                        .await
                        .in_state(PrefaceErrorState::ReadPreface)?;
                    if conn.buf.len() > 23 {
                        if conn.buf.starts_with(&PREFACE) {
                            conn.buf.split_to(24);
                            break HandshakeState::SendPreface(conn);
                        } else {
                            Err(PrefaceError::InvalidPreface)?
                        }
                    }
                }
            }
            HandshakeState::SendPreface(mut conn) => {
                info!("[+] send preface to server");
                write_and_flush(&mut conn.server, &PREFACE)
                    .await
                    .in_state(PrefaceErrorState::WritePreface)?;
                HandshakeState::ReadServerSettings(conn)
            }
            HandshakeState::ReadServerSettings(conn) => {
                info!("[+] read server settings");
                let mut framed_conn = PrefaceFramed::from(conn);
                let frame = read_frame(&mut framed_conn.server_framed_reader)
                    .await
                    .in_state(PrefaceErrorState::ReadClientSettings)?;
                let settings = parse_settings_frame(frame.as_ref())
                    .map_err(PrefaceError::Frame)?;
                framed_conn.server_settings = Some(settings);
                HandshakeState::SendServerSettings(framed_conn, frame)
            }
            HandshakeState::SendServerSettings(mut framed_conn, frame) => {
                info!("[+] send server settings to client");
                write_and_flush(&mut framed_conn.client_writer, &frame)
                    .await
                    .in_state(PrefaceErrorState::PollServerSettings)?;
                HandshakeState::ReadClientSettings(framed_conn)
            }
            HandshakeState::ReadClientSettings(mut framed_conn) => {
                info!("[+] read client settings");
                let frame = read_frame(&mut framed_conn.client_framed_reader)
                    .await
                    .in_state(PrefaceErrorState::ReadClientSettings)?;
                let settings = parse_settings_frame(frame.as_ref())
                    .map_err(PrefaceError::Frame)?;
                framed_conn.client_settings = Some(settings);
                HandshakeState::SendClientSettings(framed_conn, frame)
            }
            HandshakeState::SendClientSettings(mut framed_conn, frame) => {
                info!("[+] send client settings to server");
                write_and_flush(&mut framed_conn.server_writer, &frame)
                    .await
                    .in_state(PrefaceErrorState::SendClientSettings)?;
                HandshakeState::End(framed_conn)
            }
            HandshakeState::End(_) => self,
        };

        Ok(next_state)
    }

    pub fn ended(&self) -> bool {
        matches!(self, Self::End(_))
    }
}

impl<T, E> TryFrom<HandshakeState<T, E>> for PrefaceFramed<T, E>
where
    T: AsyncRead + AsyncWrite + Unpin,
    E: AsyncRead + AsyncWrite + Unpin,
{
    type Error = ();

    fn try_from(state: HandshakeState<T, E>) -> Result<Self, Self::Error> {
        match state {
            HandshakeState::End(conn) => Ok(conn),
            _ => Err(()),
        }
    }
}

pub fn parse_settings_frame(buf: &[u8]) -> Result<Settings, frame::Error> {
    let head = Head::parse(&buf[..HEADER_LEN]);
    Settings::load(head, &buf[HEADER_LEN..])
}
*/
