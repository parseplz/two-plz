use std::io::Error;

use tokio::io::{AsyncWrite, AsyncWriteExt};

pub async fn write_and_flush<T>(
    mut stream: T,
    data: &[u8],
) -> Result<(), Error>
where
    T: AsyncWrite + Unpin,
{
    let _ = stream.write(data).await?;
    stream.flush().await
}
