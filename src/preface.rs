use bytes::{BufMut, BytesMut};
use std::{fmt, io::Error};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, ReadHalf, WriteHalf};
use tokio_util::codec::{
    FramedRead as TokioFramedRead, LengthDelimitedCodec, length_delimited,
};
use tracing::trace;

use crate::{
    frame::{self, HEADER_LEN, Settings},
    io::{read_frame, write_and_flush},
};

const PREFACE: [u8; 24] = *b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";

type RawFramedReader<T> = TokioFramedRead<ReadHalf<T>, LengthDelimitedCodec>;

#[derive(Debug, Error)]
pub enum PrefaceError {
    #[error("read preface| {0}")]
    ReadPreface(Error),
    #[error("write preface received")]
    InvalidPreface,

    #[error("write preface| {0}")]
    WritePreface(Error),
    // TODO: include in display
    #[error("frame error")]
    Frame(frame::Error),
}

struct Preface<T, E>
where
    T: AsyncRead + AsyncWrite + Unpin,
    E: AsyncRead + AsyncWrite + Unpin,
{
    client: T,
    server: E,
    buf: BytesMut,
}

impl<T, E> Preface<T, E>
where
    T: AsyncRead + AsyncWrite + Unpin,
    E: AsyncRead + AsyncWrite + Unpin,
{
    fn new(client: T, server: E) -> Preface<T, E> {
        Preface {
            client,
            server,
            buf: BytesMut::new(),
        }
    }
}

pub struct Framed<T, E>
where
    T: AsyncRead + AsyncWrite + Unpin,
    E: AsyncRead + AsyncWrite + Unpin,
{
    pub cfrx: RawFramedReader<T>,
    pub ctx: WriteHalf<T>,
    pub client_settings: Option<Settings>,
    pub sfrx: RawFramedReader<E>,
    pub stx: WriteHalf<E>,
    pub server_settings: Option<Settings>,
}

impl<T, E> Framed<T, E>
where
    T: AsyncRead + AsyncWrite + Unpin,
    E: AsyncRead + AsyncWrite + Unpin,
{
    fn build_frame_reader<U>(
        stream: U,
    ) -> TokioFramedRead<U, LengthDelimitedCodec>
    where
        U: AsyncRead + Unpin,
    {
        length_delimited::Builder::new()
            .big_endian()
            .length_field_length(3)
            .length_adjustment(9)
            .num_skip(0) // Don't skip the header
            .new_read(stream)
    }
}

impl<T, E> From<Preface<T, E>> for Framed<T, E>
where
    T: AsyncRead + AsyncWrite + Unpin,
    E: AsyncRead + AsyncWrite + Unpin,
{
    fn from(preface: Preface<T, E>) -> Self {
        // read client => to server
        let (crx, ctx) = tokio::io::split(preface.client);
        let (srx, stx) = tokio::io::split(preface.server);
        let mut cfrx: RawFramedReader<T> = Self::build_frame_reader(crx);
        cfrx.read_buffer_mut()
            .put_slice(&preface.buf);
        let sfrx: RawFramedReader<E> = Self::build_frame_reader(srx);
        Framed {
            cfrx,
            ctx,
            client_settings: None,
            sfrx,
            stx,
            server_settings: None,
        }
    }
}

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
    ReadClientPreface(Preface<T, E>),
    SendPrefaceToServer(Preface<T, E>),
    // server settings => client
    ReadServerSettings(Preface<T, E>),
    SendServerSettingsToClient(Framed<T, E>, BytesMut),
    // client settings => server
    ReadClientSettings(Framed<T, E>),
    SendClientSettingsToServer(Framed<T, E>, BytesMut),
    // Client - server settings ack => server
    ReadServerSettingsAckFromClient(Framed<T, E>),
    SendServerSettingsAckToServer(Framed<T, E>, BytesMut),
    // Server - client settings ack => client
    ReadClientSettingsAckFromServer(Framed<T, E>),
    SendClientSettingsAckToClient(Framed<T, E>, BytesMut),
    End(Framed<T, E>),
    EndedWithoutAck(Framed<T, E>),
}

impl<T, E> HandshakeState<T, E>
where
    T: AsyncRead + AsyncWrite + Unpin,
    E: AsyncRead + AsyncWrite + Unpin,
{
    pub fn new(client: T, server: E) -> HandshakeState<T, E> {
        HandshakeState::ReadClientPreface(Preface::new(client, server))
    }

    pub async fn next(self) -> Result<Self, PrefaceError> {
        let next_state = match self {
            HandshakeState::ReadClientPreface(mut conn) => {
                trace!("[+] read client preface");
                loop {
                    let _ = conn
                        .client
                        .read_buf(&mut conn.buf)
                        .await
                        .map_err(PrefaceError::ReadPreface)?;
                    if conn.buf.len() > 23 {
                        if conn.buf.starts_with(&PREFACE) {
                            conn.buf.split_to(24);
                            break HandshakeState::SendPrefaceToServer(conn);
                        } else {
                            Err(PrefaceError::InvalidPreface)?
                        }
                    }
                }
            }
            HandshakeState::SendPrefaceToServer(mut conn) => {
                trace!("[+] send preface to server");
                write_and_flush(&mut conn.server, &PREFACE)
                    .await
                    .map_err(PrefaceError::WritePreface)?;
                HandshakeState::ReadServerSettings(conn)
            }
            /*

                HandshakeState::ReadServerSettings(conn) => {
                    trace!("[+] read server settings");
                    let mut framed_conn = Framed::from(conn);
                    let frame =
                        read_frame(&mut framed_conn.sfrx, Role::Server).await?;
                    let settings = parse_settings_frame(frame.as_ref())
                        .map_err(PrefaceError::Frame)?;
                    framed_conn.server_settings = Some(settings);
                    HandshakeState::SendServerSettingsToClient(framed_conn, frame)
                }
                HandshakeState::SendServerSettingsToClient(
                    mut framed_conn,
                    frame,
                ) => {
                    trace!("[+] send server settings to client");
                    write_and_flush(&mut framed_conn.ctx, &frame, Role::Client)
                        .await?;
                    HandshakeState::ReadClientSettings(framed_conn)
                }
                HandshakeState::ReadClientSettings(mut framed_conn) => {
                    trace!("[+] read client settings");
                    let frame =
                        read_frame(&mut framed_conn.cfrx, Role::Client).await?;
                    let settings = parse_settings_frame(frame.as_ref())
                        .map_err(PrefaceError::Frame)?;
                    framed_conn.client_settings = Some(settings);
                    HandshakeState::SendClientSettingsToServer(framed_conn, frame)
                }
                HandshakeState::SendClientSettingsToServer(
                    mut framed_conn,
                    frame,
                ) => {
                    trace!("[+] send client settings to server");
                    write_and_flush(&mut framed_conn.stx, &frame, Role::Server)
                        .await?;
                    HandshakeState::ReadServerSettingsAckFromClient(framed_conn)
                }
                HandshakeState::ReadServerSettingsAckFromClient(
                    mut framed_conn,
                ) => {
                    trace!("[+] read client settings ack");
                    let frame =
                        read_frame(&mut framed_conn.cfrx, Role::Client).await?;
                    let head = frame::Head::parse(&frame[..HEADER_LEN]);
                    if head.kind() == Kind::Settings {
                        let settings = Settings::load(head, &frame[HEADER_LEN..])
                            .map_err(PrefaceError::Frame)?;
                        if settings.is_ack() {
                            return Ok(
                                HandshakeState::SendServerSettingsAckToServer(
                                    framed_conn,
                                    frame,
                                ),
                            );
                        } else {
                            // maybe client sends another settings frame ?
                            // add frame to buffer
                        }
                    } else {
                        // add frame to buffer
                    }
                    HandshakeState::EndedWithoutAck(framed_conn)
                }
                HandshakeState::SendServerSettingsAckToServer(
                    mut framed_conn,
                    frame,
                ) => {
                    trace!("[+] send client settings ack to server");
                    write_and_flush(&mut framed_conn.stx, &frame, Role::Server)
                        .await?;
                    Self::End(framed_conn)
                }
            */
            _ => self,
        };

        Ok(next_state)
    }

    pub fn ended(&self) -> bool {
        matches!(self, Self::End(_))
            || matches!(self, Self::EndedWithoutAck(_))
    }
}

impl<T, E> TryFrom<HandshakeState<T, E>> for Framed<T, E>
where
    T: AsyncRead + AsyncWrite + Unpin,
    E: AsyncRead + AsyncWrite + Unpin,
{
    type Error = ();

    fn try_from(state: HandshakeState<T, E>) -> Result<Self, Self::Error> {
        match state {
            HandshakeState::End(conn)
            | HandshakeState::EndedWithoutAck(conn) => Ok(conn),
            _ => Err(()),
        }
    }
}

pub fn parse_settings_frame(buf: &[u8]) -> Result<Settings, frame::Error> {
    let head = frame::Head::parse(&buf[..HEADER_LEN]);
    Settings::load(head, &buf[HEADER_LEN..])
}
