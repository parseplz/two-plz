use futures::StreamExt;
use support::{build_test_request, prelude::*};

#[tokio::test]
async fn recv_trailers_only() {
    support::trace_init!();

    let mock = mock_io::Builder::new()
        .handshake()
        // Write GET /
        .write(&[
            0, 0, 0x10, 1, 5, 0, 0, 0, 1, 0x82, 0x87, 0x41, 0x8B, 0x9D, 0x29,
            0xAC, 0x4B, 0x8F, 0xA8, 0xE9, 0x19, 0x97, 0x21, 0xE9, 0x84,
        ])
        // Read response
        .read(&[
            0, 0, 1, 1, 4, 0, 0, 0, 1, 0x88, 0, 0, 9, 1, 5, 0, 0, 0, 1, 0x40,
            0x84, 0x42, 0x46, 0x9B, 0x51, 0x82, 0x3F, 0x5F,
        ])
        .build();

    let (mut conn, mut client) = ClientBuilder::new()
        .handshake(mock)
        .await
        .unwrap();

    // Send the request
    let request = build_test_request();
    let resp_fut = client.send_request(request).unwrap();
    let resp: Response = conn.run(resp_fut).await.unwrap();
    let trailers = resp.trailers().unwrap();
    assert_eq!(trailers.len(), 1);
    assert_eq!(trailers["status"], "ok");
    conn.await.unwrap();
}

#[tokio::test]
async fn send_trailers_immediately() {
    support::trace_init!();

    let mock = mock_io::Builder::new()
        .handshake()
        // Write GET /
        .write(&[
            0, 0, 0x10, 1, 4, 0, 0, 0, 1, 0x82, 0x87, 0x41, 0x8B, 0x9D, 0x29,
            0xAC, 0x4B, 0x8F, 0xA8, 0xE9, 0x19, 0x97, 0x21, 0xE9, 0x84, 0, 0,
            0x0A, 1, 5, 0, 0, 0, 1, 0x40, 0x83, 0xF6, 0x7A, 0x66, 0x84, 0x9C,
            0xB4, 0x50, 0x7F,
        ])
        // Read response
        .read(&[
            0, 0, 1, 1, 4, 0, 0, 0, 1, 0x88, 0, 0, 0x0B, 0, 1, 0, 0, 0, 1,
            0x68, 0x65, 0x6C, 0x6C, 0x6F, 0x20, 0x77, 0x6F, 0x72, 0x6C, 0x64,
        ])
        .build();

    let (mut conn, mut client) = ClientBuilder::new()
        .handshake(mock)
        .await
        .unwrap();

    // Send the request
    let mut request = build_test_request();
    let mut trailers = HeaderMap::new();
    trailers.insert("zomg", "hello".parse().unwrap());
    request.set_trailer(trailers);

    let resp_fut = client.send_request(request).unwrap();
    let resp: Response = conn.run(resp_fut).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.body_as_ref(), Some(&"hello world"[..].into()));
    assert!(resp.trailers().is_none());

    conn.await.unwrap();
}
