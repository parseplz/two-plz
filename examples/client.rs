#![allow(warnings)]
extern crate two_plz;

use header_plz::{
    Method,
    uri::{Uri, scheme::Scheme},
};
use http_plz::{OneResponse, Request};
use rustls_pki_types::ServerName;
use tokio::{net::TcpStream, task};
use tracing::{Level, level_filters::LevelFilter};
use tracing_subscriber::{
    filter, layer::SubscriberExt, util::SubscriberInitExt,
};
use two_plz::client::ClientBuilder;

mod encrypt;
use encrypt::captain_crypto::CaptainCrypto;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let filter = filter::Targets::new()
        .with_target("two_plz", Level::INFO)
        .with_target("rustls", LevelFilter::OFF);
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(filter)
        .init();
    let captain_crypto = CaptainCrypto::new()?;
    let server_tcp = TcpStream::connect("www.google.com:443").await?;
    let sni = ServerName::try_from("www.google.com")?;
    let connector = captain_crypto.get_connector();
    let server_tls = connector
        .connect(sni, server_tcp)
        .await?;
    let (conn, mut handler) = ClientBuilder::new()
        .handshake(server_tls)
        .await?;

    task::spawn(async move {
        if let Err(e) = conn.await {
            eprintln!("{}", e);
        }
    });

    let uri = Uri::builder()
        .scheme(Scheme::HTTPS)
        .authority("www.google.com")
        .path("/robots.txt")
        .build()
        .unwrap();

    let request = Request::builder()
        .method(Method::GET)
        .uri(uri)
        .build();

    let resp_fut = handler.send_request(request)?;
    let resp = resp_fut.await.unwrap();
    println!("##### response #####");
    println!();
    println!("{:#?}", resp);
    println!();

    println!("##### response as h11 #####");
    println!();
    let buf = OneResponse::from(resp).into_bytes();
    println!("{:#?}", buf);

    Ok(())
}
