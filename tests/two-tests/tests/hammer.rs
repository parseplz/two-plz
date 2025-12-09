use futures::{FutureExt, StreamExt};
use support::prelude::*;

use std::io;
use std::{
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use tokio::net::{TcpListener, TcpStream};

struct Server {
    addr: SocketAddr,
    reqs: Arc<AtomicUsize>,
    _join: Option<thread::JoinHandle<()>>,
}

impl Server {
    fn serve<F>(mk_data: F) -> Self
    where
        F: Fn() -> Bytes,
        F: Send + Sync + 'static,
    {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let listener = rt
            .block_on(TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0))))
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let reqs = Arc::new(AtomicUsize::new(0));
        let reqs2 = reqs.clone();
        let join = thread::spawn(move || {
            let server = async move {
                loop {
                    let socket = listener.accept().await.map(|(s, _)| s);
                    let reqs = reqs2.clone();
                    tokio::spawn(async move {
                        if let Err(e) = handle_request(socket, reqs).await {
                            eprintln!("serve conn error: {:?}", e)
                        }
                    });
                }
            };

            rt.block_on(server);
        });

        Self {
            addr,
            _join: Some(join),
            reqs,
        }
    }

    fn addr(&self) -> SocketAddr {
        self.addr
    }

    fn request_count(&self) -> usize {
        self.reqs.load(Ordering::Acquire)
    }
}

async fn handle_request(
    socket: io::Result<TcpStream>,
    reqs: Arc<AtomicUsize>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut conn = ServerBuilder::new()
        .handshake(socket?)
        .await?;
    while let Some(result) = conn.next().await {
        let (_req, mut respond) = result.unwrap();
        reqs.fetch_add(1, Ordering::Release);

        let scode = ResponseLine {
            status: StatusCode::from_u16(200).unwrap(),
        };
        let body = BytesMut::from("hello world");
        let resp = Response::new(scode, HeaderMap::new(), Some(body), None);
        respond.send_response(resp)?;
    }
    Ok(())
}

#[ignore]
#[test]
fn hammer_client_concurrency() {
    // This reproduces issue #326.
    const N: usize = 5000;

    let server = Server::serve(|| Bytes::from_static(b"hello world!"));

    let addr = server.addr();
    let rsps = Arc::new(AtomicUsize::new(0));

    for i in 0..N {
        print!("sending {}", i);
        let rsps = rsps.clone();
        let tcp = TcpStream::connect(&addr);
        let tcp = tcp
            .then(|stream| {
                let b = ClientBuilder::new();
                b.handshake(stream.unwrap())
            })
            .then(move |res| {
                let rsps = rsps;
                let (conn, mut client) = res.unwrap();
                let request = build_test_request();
                let response = client.send_request(request).unwrap();

                tokio::spawn(async move {
                    conn.await.unwrap();
                });

                response
                    .map(|r| assert_eq!(r.unwrap().status(), StatusCode::OK))
                    .map(move |_| {
                        rsps.fetch_add(1, Ordering::Release);
                    })
            });

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(tcp);
        println!("...done");
    }

    println!("all done");

    assert_eq!(N, rsps.load(Ordering::Acquire));
    assert_eq!(N, server.request_count());
}
