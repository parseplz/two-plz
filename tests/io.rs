
use bytes::BytesMut;
use futures::StreamExt;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio_util::codec::{FramedRead, LengthDelimitedCodec};

use crate::{IoError, IoOp, Role};

pub(crate) async fn write_and_flush<T>(
    mut stream: T,
    data: &[u8],
    role: Role,
) -> Result<(), IoError>
where
    T: AsyncWrite + Unpin,
{
    let _ = stream
        .write(data)
        .await
        .map_err(|e| IoError::new(Role::Client, IoOp::Write, e));
    stream
        .flush()
        .await
        .map_err(|e| IoError::new(Role::Client, IoOp::Flush, e))
}

pub(crate) async fn read_frame<T>(
    stream: &mut FramedRead<T, LengthDelimitedCodec>,
    role: Role,
) -> Result<BytesMut, IoError>
where
    T: AsyncRead + Unpin,
{
    loop {
        if let Some(frame) = stream.next().await {
            return frame.map_err(|e| IoError::new(role, IoOp::Read, e));
        } else {
            continue;
        }
    }
}
