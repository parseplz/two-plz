pub mod captain_crypto;
use std::sync::Arc;

use bytes::BytesMut;
use rustls::pki_types::ServerName;
use rustls::server::Acceptor;
use rustls_pki_types::{CertificateDer, InvalidDnsNameError};
use tokio::net::TcpStream;
use tokio_rustls::{
    LazyConfigAcceptor, StartHandshake, TlsConnector, client, server,
};

const ALPN_H1: &[u8] = b"http/1.1";
const OWS: char = ' ';

// 1
pub async fn perform_handshake(
    stream: TcpStream,
) -> StartHandshake<TcpStream> {
    let acceptor = Acceptor::default();
    LazyConfigAcceptor::new(acceptor, stream)
        .await
        .unwrap()
}

// 2
pub async fn encrypt_server(
    server_name: &str,
    client_stream: &StartHandshake<TcpStream>,
    server_stream: TcpStream,
    connector: Arc<TlsConnector>,
) -> Result<client::TlsStream<TcpStream>, Box<dyn std::error::Error>> {
    let client_hello = client_stream.client_hello();
    let sni = client_hello.server_name();
    let parsed_sni = parse_sni(sni, server_name)?.to_owned();
    connector
        .connect(parsed_sni.clone(), server_stream)
        .await
        .map_err(Into::into)
}

// 3
pub async fn complete_handshake(
    client_stream: StartHandshake<TcpStream>,
    server_stream: &client::TlsStream<TcpStream>,
    ca: &mut captain_crypto::CaptainCrypto,
) -> server::TlsStream<TcpStream> {
    let cert_chain = server_stream
        .get_ref()
        .1
        .peer_certificates()
        .unwrap();
    let owned_cert_chain: Vec<CertificateDer<'static>> = cert_chain
        .iter()
        .cloned()
        .map(|x| x.into_owned())
        .collect();
    let config = ca
        .generate_new_cert(true, owned_cert_chain)
        .unwrap();
    client_stream
        .into_stream(config)
        .await
        .unwrap()
}

pub fn get_hostname(mut data: BytesMut) -> String {
    let mut index = data
        .iter()
        .position(|&x| x == OWS as u8)
        .unwrap();
    let _ = data.split_to(index + 1);
    // 2. Second OWS
    index = data
        .iter()
        .position(|&x| x == OWS as u8)
        .unwrap();
    let uri = data.split_to(index);
    String::from_utf8_lossy(&uri).to_string()
}

pub fn parse_sni<'a>(
    sni: Option<&'a str>,
    server_name: &'a str,
) -> Result<ServerName<'a>, InvalidDnsNameError> {
    if let Some(sni) = sni {
        ServerName::try_from(sni)
    } else {
        ServerName::try_from(server_name)
    }
}
