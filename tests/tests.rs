use bytes::BytesMut;
use tokio::{io::AsyncReadExt, net::TcpListener};
mod io;

const CONNECTION_ESTABLISHED: [u8; 39] =
    *b"HTTP/1.1 200 Connection Established\r\n\r\n";

#[tokio::test]
async fn mock_server() -> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind("127.0.0.1:8080").await?;
    let (mut client_tcp, _) = listener.accept().await?;

    let mut buf = BytesMut::new();
    let _ = client_tcp.read_buf(&mut buf).await?;
    let _ = client_tcp.read_buf(&mut buf).await?;

    dbg!(buf);
    Ok(())
}
