use std::sync::Arc;

use bytes::BytesMut;
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadHalf, WriteHalf},
    sync::Mutex,
};

use crate::{
    codec::framed_read::HttpFramedRead,
    frame::{self, Settings},
    preface::PrefaceFramed,
    proto::{buffer::Buffer, conn_settings::ConnSettings, store::Store},
};

pub struct Connection<T, E> {
    reader: HttpFramedRead<ReadHalf<T>>,
    writer: WriteHalf<E>,
    settings: Arc<Mutex<ConnSettings>>,
    store: Store,
    buf: Buffer<BytesMut>,
}

impl<T, E> From<PrefaceFramed<T, E>> for (Connection<T, E>, Connection<E, T>)
where
    T: AsyncRead + Unpin,
    E: AsyncRead + Unpin,
{
    fn from(mut framed: PrefaceFramed<T, E>) -> Self {
        let settings = Arc::new(Mutex::new(ConnSettings::from(&mut framed)));
        let client = Connection {
            reader: HttpFramedRead::new(framed.client_framed_reader),
            writer: framed.server_writer,
            store: Store::new(),
            buf: Buffer::new(),
            ..todo!()
        };
        let server = Connection {
            reader: HttpFramedRead::new(framed.server_framed_reader),
            writer: framed.client_writer,
            store: Store::new(),
            buf: Buffer::new(),
            ..todo!()
        };
        (client, server)
    }
}
