use crate::proto::Error as ProtoError;
use crate::{
    codec::framed_read::build_raw_frame_reader,
    frame::{self, HEADER_LEN, Head, Settings},
};
use bytes::{BufMut, BytesMut};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, ReadHalf, WriteHalf};
use tokio_util::codec::{
    FramedRead as TokioFramedRead, LengthDelimitedCodec,
};
use tracing::trace;

use crate::io::{read_frame, write_and_flush};

const PREFACE: [u8; 24] = *b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";

#[derive(Debug, Error)]
pub enum PrefaceErrorState {
    #[error("read preface")]
    ReadPreface,
    #[error("write preface")]
    WritePreface,
    #[error("read server settings")]
    ReadServerSettings,
    #[error("send server settings")]
    SendServerSettings,
    #[error("read client settings")]
    ReadClientSettings,
    #[error("send client settings")]
    SendClientSettings,
}

trait IoStateExt<T> {
    fn in_state(self, state: PrefaceErrorState) -> Result<T, PrefaceError>;
}

impl<T> IoStateExt<T> for Result<T, std::io::Error> {
    #[inline]
    fn in_state(self, state: PrefaceErrorState) -> Result<T, PrefaceError> {
        self.map_err(|e| PrefaceError::Proto(state, e.into()))
    }
}

#[derive(Debug, Error)]
pub enum PrefaceError {
    #[error("io error| {0}| {1}")]
    Proto(PrefaceErrorState, ProtoError),
    #[error("write preface received")]
    InvalidPreface,
    // TODO: include in display
    #[error("frame error")]
    Frame(frame::Error),
}

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
        // read client => to server
        let (client_rx, client_tx) = tokio::io::split(preface.client);
        let (server_rx, server_tx) = tokio::io::split(preface.server);
        let mut client_framed_reader = build_raw_frame_reader(client_rx);
        client_framed_reader
            .read_buffer_mut()
            .put_slice(&preface.buf);
        let server_framed_reader = build_raw_frame_reader(server_rx);
        PrefaceFramed {
            client_framed_reader,
            client_writer: client_tx,
            client_settings: None,
            is_client_settings_ack: false,
            client_frame: None,
            server_framed_reader,
            server_writer: server_tx,
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

    pub async fn next(self) -> Result<Self, PrefaceError> {
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
                    .in_state(PrefaceErrorState::ReadClientSettings)?;
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
