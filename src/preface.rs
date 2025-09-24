use crate::frame::{self, HEADER_LEN, Head, Kind, Settings};
use bytes::{BufMut, BytesMut};
use std::{fmt, io::Error};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, ReadHalf, WriteHalf};
use tokio_util::codec::{
    FramedRead as TokioFramedRead, LengthDelimitedCodec, length_delimited,
};
use tracing::trace;

use crate::io::{read_frame, write_and_flush};

const PREFACE: [u8; 24] = *b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";

type RawFramedReader<T> = TokioFramedRead<ReadHalf<T>, LengthDelimitedCodec>;

#[derive(Debug)]
pub enum PrefaceErrorState {
    ReadPreface,
    WritePreface,
    ReadServerSettings,
    SendServerSettings,
    ReadClientSettings,
    SendClientSettings,
}

trait IoStateExt<T> {
    fn in_state(self, state: PrefaceErrorState) -> Result<T, PrefaceError>;
}

impl<T> IoStateExt<T> for Result<T, std::io::Error> {
    #[inline]
    fn in_state(self, state: PrefaceErrorState) -> Result<T, PrefaceError> {
        self.map_err(|e| PrefaceError::Io(state, e))
    }
}

impl fmt::Display for PrefaceErrorState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PrefaceErrorState::ReadPreface => write!(f, "read preface"),
            _ => todo!(),
        }
    }
}

#[derive(Debug, Error)]
pub enum PrefaceError {
    #[error("io error| {0}| {1}")]
    Io(PrefaceErrorState, Error),
    #[error("write preface received")]
    InvalidPreface,
    // TODO: include in display
    #[error("frame error")]
    Frame(frame::Error),
}

pub struct Preface<T, E>
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

pub struct PrefaceFramed<T, E>
where
    T: AsyncRead + AsyncWrite + Unpin,
    E: AsyncRead + AsyncWrite + Unpin,
{
    pub client_framed_reader: RawFramedReader<T>,
    pub client_writer: WriteHalf<T>,
    pub client_settings: Option<Settings>,
    pub is_client_settings_ack: bool,
    pub client_frame: Option<BytesMut>,
    pub server_framed_reader: RawFramedReader<E>,
    pub server_writer: WriteHalf<E>,
    pub server_settings: Option<Settings>,
    pub is_server_settings_ack: bool,
}

impl<T, E> PrefaceFramed<T, E>
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

impl<T, E> From<Preface<T, E>> for PrefaceFramed<T, E>
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
        PrefaceFramed {
            client_framed_reader: cfrx,
            client_writer: ctx,
            client_settings: None,
            is_client_settings_ack: false,
            client_frame: None,
            server_framed_reader: sfrx,
            server_writer: stx,
            server_settings: None,
            is_server_settings_ack: false,
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

    pub async fn next(mut self) -> Result<Self, PrefaceError> {
        let next_state = match self {
            HandshakeState::ReadPreface(mut conn) => {
                trace!("[+] read client preface");
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
                trace!("[+] send preface to server");
                write_and_flush(&mut conn.server, &PREFACE)
                    .await
                    .in_state(PrefaceErrorState::WritePreface)?;
                HandshakeState::ReadServerSettings(conn)
            }
            HandshakeState::ReadServerSettings(conn) => {
                trace!("[+] read server settings");
                let mut framed_conn = PrefaceFramed::from(conn);
                let frame = read_frame(&mut framed_conn.server_framed_reader)
                    .await
                    .in_state(PrefaceErrorState::ReadServerSettings)?;
                let settings = parse_settings_frame(frame.as_ref())
                    .map_err(PrefaceError::Frame)?;
                framed_conn.server_settings = Some(settings);
                HandshakeState::SendServerSettings(framed_conn, frame)
            }
            HandshakeState::SendServerSettings(mut framed_conn, frame) => {
                trace!("[+] send server settings to client");
                write_and_flush(&mut framed_conn.client_writer, &frame)
                    .await
                    .in_state(PrefaceErrorState::SendServerSettings)?;
                HandshakeState::ReadClientSettings(framed_conn)
            }
            HandshakeState::ReadClientSettings(mut framed_conn) => {
                trace!("[+] read client settings");
                let frame = read_frame(&mut framed_conn.server_framed_reader)
                    .await
                    .in_state(PrefaceErrorState::ReadClientSettings)?;
                let settings = parse_settings_frame(frame.as_ref())
                    .map_err(PrefaceError::Frame)?;
                framed_conn.client_settings = Some(settings);
                HandshakeState::SendClientSettings(framed_conn, frame)
            }
            HandshakeState::SendClientSettings(mut framed_conn, frame) => {
                trace!("[+] send client settings to server");
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
