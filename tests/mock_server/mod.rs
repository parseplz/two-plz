extern crate two_plz;
use bytes::BytesMut;
use rustls_pki_types::ServerName;
use tokio::{
    io::AsyncReadExt,
    net::{TcpListener, TcpStream},
};
use tracing::{Level, info};
use two_plz::{
    builder::{ClientBuilder, ServerBuilder},
    io::write_and_flush,
    preface::PrefaceState,
};

mod encrypt;
use encrypt::{
    captain_crypto::CaptainCrypto, complete_handshake, encrypt_server,
    get_hostname, perform_handshake,
};

const CONNECTION_ESTABLISHED: [u8; 39] =
    *b"HTTP/1.1 200 Connection Established\r\n\r\n";

//#[tokio::test]
async fn mock_server() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_max_level(Level::DEBUG)
        .init();

    let mut captain_crypto = CaptainCrypto::new()?;
    let listener = TcpListener::bind("127.0.0.1:8080").await?;
    let (mut client_tcp, _) = listener.accept().await?;

    let mut buf = BytesMut::new();
    let _ = client_tcp.read_buf(&mut buf).await?;
    let _ = client_tcp.read_buf(&mut buf).await?;

    let server_addr = get_hostname(buf.split());

    write_and_flush(&mut client_tcp, &CONNECTION_ESTABLISHED).await?;

    // perform handshake
    let client_handshake = perform_handshake(client_tcp).await;

    // establish server connection
    let server_tcp = TcpStream::connect(&server_addr).await?;

    // encrypt server
    let connector = captain_crypto.get_connector();
    let server_tls =
        encrypt_server(&server_addr, &client_handshake, server_tcp, connector)
            .await?;

    let alpn = server_tls
        .get_ref()
        .1
        .alpn_protocol()
        .unwrap_or_default();
    info!("[+] server alpn| {}", String::from_utf8_lossy(alpn));

    // complete tls handshake
    let client_tls =
        complete_handshake(client_handshake, &server_tls, &mut captain_crypto)
            .await;

    let alpn = client_tls
        .get_ref()
        .1
        .alpn_protocol()
        .unwrap_or_default();
    info!("[+] client alpn| {}", String::from_utf8_lossy(alpn));

    let (conn, handler) = ServerBuilder::new()
        .handshake(client_tls)
        .await?;

    Ok(())
}

#[tokio::test]
async fn mock_client() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_max_level(Level::DEBUG)
        .init();
    let mut captain_crypto = CaptainCrypto::new()?;
    let server_tcp = TcpStream::connect("www.google.com:443").await?;
    let sni = ServerName::try_from("www.google.com")?;
    let connector = captain_crypto.get_connector();
    let server_tls = connector
        .connect(sni, server_tcp)
        .await?;
    let (conn, handler) = ClientBuilder::new()
        .handshake(server_tls)
        .await?;

    Ok(())
}
