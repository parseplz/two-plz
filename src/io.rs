use std::io::Error;

use bytes::BytesMut;
use futures::StreamExt;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio_util::codec::{FramedRead, LengthDelimitedCodec};

pub(crate) async fn read_frame<T>(
    stream: &mut FramedRead<T, LengthDelimitedCodec>,
) -> Result<BytesMut, Error>
where
    T: AsyncRead + Unpin,
{
    loop {
        if let Some(frame) = stream.next().await {
            return frame;
        } else {
            continue;
        }
    }
}

pub(crate) async fn write_and_flush<T>(
    mut stream: T,
    data: &[u8],
) -> Result<(), Error>
where
    T: AsyncWrite + Unpin,
{
    let _ = stream.write(data).await?;
    stream.flush().await
}
