
use bytes::BytesMut;
use futures::StreamExt;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio_util::codec::{FramedRead, LengthDelimitedCodec};

pub(crate) async fn read_frame<T>(
    stream: &mut FramedRead<T, LengthDelimitedCodec>,
) -> Result<BytesMut, std::io::Error>
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
