#![allow(warnings)]
extern crate two_plz;

use bytes::BytesMut;
use futures::{StreamExt, stream::FuturesUnordered};
use header_plz::{
    Method,
    uri::{Uri, scheme::Scheme},
};
use http_plz::{OneResponse, Request};
use rustls_pki_types::ServerName;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    task,
};
use tracing::{Level, level_filters::LevelFilter};
use tracing_subscriber::{
    filter, layer::SubscriberExt, util::SubscriberInitExt,
};
use two_plz::spa::Mode;
use two_plz::{
    client::{ClientBuilder, poll_once},
    server::ServerBuilder,
};

mod encrypt;
use encrypt::captain_crypto::CaptainCrypto;

const N: u8 = 6;

// cargo r --example server
// cargo r --example spa -- --mode basic --requests get
//
// modes
//  - basic
//  - standard_ping
//  - advanced_ping
//
// requests
//  - get
//  - post
//  - mixed
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let mode = match args
        .get(2)
        .map(|s| s.as_str())
        .unwrap_or_default()
    {
        "standard" => Mode::ping(),
        "enhanced" => Mode::enhanced(),
        _ => Mode::default(),
    };

    let req_close = match args
        .get(4)
        .map(|s| s.as_str())
        .unwrap_or_default()
    {
        "get" => build_all_get_requests,
        "mixed" => build_mixed_requests,
        _ => build_all_post_requests,
    };

    let filter = filter::Targets::new()
        .with_target("two_plz", Level::INFO)
        .with_target("rustls", LevelFilter::OFF);
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(filter)
        .init();

    let captain_crypto = CaptainCrypto::new()?;
    let mut server_tcp = TcpStream::connect("127.0.0.1:8080").await?;
    server_tcp.set_nodelay(true);
    server_tcp
        .write_all(
            b"CONNECT www.google.com:443 HTTP/1.1\r\n\
            Host: www.google.com:443\r\n\
            Proxy-Connection: Keep-Alive\r\n\r\n
            ",
        )
        .await?;
    server_tcp.flush().await?;

    // read connection established
    let mut buf = BytesMut::new();
    server_tcp.read_buf(&mut buf).await;
    let sni = ServerName::try_from("www.google.com")?;
    let connector = captain_crypto.get_connector();
    let server_tls = connector
        .connect(sni, server_tcp)
        .await?;

    let is_enhanced = matches!(mode, Mode::EnhancedPing(..));

    let (mut conn, mut handler) = ClientBuilder::new()
        .single_packet_attack_mode(mode)
        .handshake(server_tls)
        .await?;

    if is_enhanced {
        poll_once(&mut conn).await.unwrap();
    }

    task::spawn(async move {
        if let Err(e) = conn.await {
            eprintln!("{}", e);
        }
    });

    let requests = req_close();

    let results: Vec<_> = handler
        .spa(requests)
        .into_iter()
        .filter_map(Result::ok)
        .collect::<FuturesUnordered<_>>()
        .collect()
        .await;

    assert_eq!(results.len(), N as usize);
    for res in results {
        assert_eq!(*res.unwrap().status(), 200);
    }
    Ok(())
}

fn build_all_post_requests() -> Vec<Request> {
    let uri =
        Uri::from_shared("https://www.google.com/robots.txt".into()).unwrap();

    let request = Request::builder()
        .method(Method::POST)
        .uri(uri)
        .body(BytesMut::from("hello"))
        .build();

    let mut requests = Vec::with_capacity(N as usize);
    for _ in 0..N {
        requests.push(request.clone());
    }
    requests
}

fn build_all_get_requests() -> Vec<Request> {
    let uri =
        Uri::from_shared("https://www.google.com/robots.txt".into()).unwrap();
    let request = Request::builder()
        .method(Method::GET)
        .uri(uri)
        .build();
    let mut requests = Vec::with_capacity(N as usize);
    for _ in 0..N {
        requests.push(request.clone());
    }
    requests
}

fn build_mixed_requests() -> Vec<Request> {
    let uri =
        Uri::from_shared("https://www.google.com/robots.txt".into()).unwrap();
    let mut requests = Vec::with_capacity(N as usize);

    let post_req = Request::builder()
        .method(Method::POST)
        .uri(uri.clone())
        .body(BytesMut::from("hello"))
        .build();

    let get_req = Request::builder()
        .method(Method::GET)
        .uri(uri)
        .build();

    for i in 0..N {
        if i % 2 == 0 {
            requests.push(post_req.clone());
        } else {
            requests.push(get_req.clone());
        }
    }
    requests
}
