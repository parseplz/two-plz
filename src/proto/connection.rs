use crate::{frame::Frame, proto::Error as ProtoError};
use std::sync::Arc;

use bytes::BytesMut;
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadHalf, WriteHalf},
    sync::Mutex,
};

use crate::{
    codec::framed_read::FramedRead,
    frame::{self, Settings},
    preface::PrefaceFramed,
    proto::{buffer::Buffer, conn_settings::ConnSettings, store::Store},
};

pub struct Connection<T, E> {
    reader: FramedRead<ReadHalf<T>>,
    writer: WriteHalf<E>,
    curr_frame: Option<Frame>,
    settings: Arc<Mutex<ConnSettings>>,
    store: Store,
    recv_buf: Buffer<BytesMut>,
    to_send: Buffer<Frame>,
}

impl<T, E> Connection<T, E>
where
    T: AsyncRead + Unpin,
{
    pub async fn read_frame(&mut self) -> Result<Frame, ProtoError> {
        self.reader.read_frame().await
    }

    pub fn set_curr_frame(&mut self, frame: Frame) {
        self.curr_frame = Some(frame)
    }

    pub fn curr_frame(&self) -> &Frame {
        self.curr_frame.as_ref().unwrap()
    }
}

impl<T, E> From<PrefaceFramed<T, E>> for (Connection<T, E>, Connection<E, T>)
where
    T: AsyncRead + Unpin,
    E: AsyncRead + Unpin,
{
    fn from(mut framed: PrefaceFramed<T, E>) -> Self {
        let settings = Arc::new(Mutex::new(ConnSettings::from(&mut framed)));
        let client = Connection {
            reader: FramedRead::new(framed.client_framed_reader),
            curr_frame: None,
            writer: framed.server_writer,
            store: Store::new(),
            recv_buf: Buffer::new(),
            settings: settings.clone(),
            to_send: Buffer::new(),
        };
        let server = Connection {
            reader: FramedRead::new(framed.server_framed_reader),
            curr_frame: None,
            writer: framed.client_writer,
            store: Store::new(),
            recv_buf: Buffer::new(),
            settings,
            to_send: Buffer::new(),
        };
        (client, server)
    }
}
