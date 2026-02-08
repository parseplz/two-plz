// run client:
// curl https://www.google.com/robots.txt -x http://localhost:8080 -ki

#![allow(warnings)]
mod encrypt;

use bytes::BytesMut;
use header_plz::{HeaderMap, status::StatusCode};
use http_plz::OneRequest;
use http_plz::Response;
use tokio::{
    io::AsyncReadExt,
    net::{TcpListener, TcpStream},
};
use tokio_rustls::server::TlsStream;
use tracing::{Level, info, level_filters::LevelFilter};
use tracing_subscriber::{
    filter, layer::SubscriberExt, util::SubscriberInitExt,
};
use two_plz::server::ServerBuilder;
use two_plz::server::ServerConnection;

use encrypt::{
    captain_crypto::CaptainCrypto, complete_handshake, encrypt_server,
    get_hostname, perform_handshake,
};

const CONNECTION_ESTABLISHED: [u8; 39] =
    *b"HTTP/1.1 200 Connection Established\r\n\r\n";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let filter = filter::Targets::new()
        .with_target("two_plz", Level::INFO)
        .with_target("rustls", LevelFilter::OFF);
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(filter)
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

    let mut server: ServerConnection<TlsStream<TcpStream>> =
        ServerBuilder::new()
            .handshake(client_tls)
            .await?;
    info!("[+] hshake done");
    while let Some(req) = server.accept().await {
        let (request, mut responder) = req.unwrap();

        println!("##### request #####");
        println!();
        println!("{:#?}", request);
        println!();

        println!("##### request as h11 #####");
        println!();
        let buf = OneRequest::from(request).into_bytes();
        println!("{:#?}", buf);

        let mut headers = HeaderMap::new();
        headers.insert("response", "header");
        headers.insert("response", "header"); // duplicate header

        // large body
        let mut body = BytesMut::from(
            "hello world"
                .repeat(1_000_000)
                .as_bytes(),
        );
        body.extend_from_slice(b"dead body");
        let response = Response::builder()
            .status(200)
            .headers(headers)
            .body(BytesMut::from("hello world"))
            .build()
            .unwrap();
        responder
            .send_response(response)
            .unwrap();
    }

    Ok(())
}

async fn write_and_flush<T>(
    mut stream: T,
    data: &[u8],
) -> Result<(), std::io::Error>
where
    T: tokio::io::AsyncWriteExt + std::marker::Unpin,
{
    stream.write_all(data).await?;
    stream.flush().await
}
