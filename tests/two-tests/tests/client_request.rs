use std::task::Context;

use support::{build_test_request, prelude::*};

#[tokio::test]
async fn client_handshake() {
    support::trace_init!();

    let mock = mock_io::Builder::new()
        .local_handshake()
        .build();

    let (conn, _client) = ClientBuilder::new()
        .handshake(mock)
        .await
        .unwrap();

    tracing::trace!("hands have been shook");

    // At this point, the connection should be closed
    conn.await.unwrap();
}

#[tokio::test]
async fn client_other_thread() {
    support::trace_init!();
    let (io, mut srv) = mock::new();

    let srv_fut = async move {
        let settings = srv.assert_client_handshake().await;
        assert_default_settings!(settings);
        srv.recv_frame(
            frames::headers(1)
                .request("GET", "https", "http2.akamai.com", "/")
                .eos(),
        )
        .await;
        srv.send_frame(frames::headers(1).response(200).eos())
            .await;
    };

    let client_fut = async move {
        let (conn, mut client) = ClientBuilder::new()
            .handshake(io)
            .await
            .unwrap();
        tokio::spawn(async move {
            let request = build_test_request();
            client
                .send_request(request)
                .unwrap()
                .await
                .expect("request");
        });
        conn.await.expect("h2");
    };
    join(srv_fut, client_fut).await;
}

#[tokio::test]
async fn recv_invalid_server_stream_id() {
    support::trace_init!();

    let io = mock_io::Builder::new()
        .local_handshake()
        // Write GET /
        .write(&[
            0, 0, 0x10, 1, 5, 0, 0, 0, 1, 0x82, 0x87, 0x41, 0x8B, 0x9D, 0x29,
            0xAC, 0x4B, 0x8F, 0xA8, 0xE9, 0x19, 0x97, 0x21, 0xE9, 0x84,
        ])
        ////// Read response
        .read(&[0, 0, 1, 1, 5, 0, 0, 0, 2, 137])
        ////// Write GO_AWAY
        .write(&[0, 0, 8, 7, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1])
        .build();

    let (conn, mut client) = ClientBuilder::new()
        .handshake(io)
        .await
        .unwrap();

    let request = build_test_request();
    let response = client.send_request(request).unwrap();

    // The connection errors
    assert!(conn.await.is_err());

    //// The stream errors
    assert!(response.await.is_err());
}

/*
// TODO: fix - client - initial_stream_id
#[ignore]
#[tokio::test]
async fn request_stream_id_overflows() {
    support::trace_init!();
    let (io, mut srv) = mock::new();

    let client_fut = async move {
        let (mut conn, mut client) = ClientBuilder::new()
            //.initial_stream_id(u32::MAX >> 1)
            .handshake(io)
            .await
            .unwrap();
        let request = build_test_request();
        let resp = client.send_request(request).unwrap();
        let _r = conn.drive(resp).await.unwrap();

        // TODO: Implement
        // second cannot use the next stream id, it's over
        // let poll_err = poll_fn(|cx| client.poll_ready(cx)).await.unwrap_err();
        // assert_eq!(poll_err.to_string(), "user error: stream ID overflowed");

        let request = build_test_request();
        let err = client
            .send_request(request)
            .unwrap()
            .await
            .unwrap_err();

        assert_eq!(err.to_string(), "user error: stream ID overflowed");
    };

    let srv_fut = async move {
        let settings = srv.assert_client_handshake().await;
        assert_default_settings!(settings);

        srv.recv_frame(
            frames::headers(u32::MAX >> 1)
                .request("GET", "https", "example.com", "/")
                .eos(),
        )
        .await;
        srv.send_frame(
            frames::headers(u32::MAX >> 1)
                .response(200)
                .eos(),
        )
        .await;
        idle_ms(10).await;
    };

    join(client_fut, srv_fut).await;
}
*/

#[tokio::test]
async fn client_builder_max_concurrent_streams() {
    support::trace_init!();
    let (io, mut srv) = mock::new();

    let mut settings = frame::Settings::default();
    settings.set_enable_push(false);
    settings.set_max_concurrent_streams(Some(1));

    let srv_fut = async move {
        let rcvd_settings = srv.assert_client_handshake().await;
        assert_frame_eq(settings, rcvd_settings);

        srv.recv_frame(
            frames::headers(1)
                .request("GET", "https", "http2.akamai.com", "/")
                .eos(),
        )
        .await;
        srv.send_frame(frames::headers(1).response(200).eos())
            .await;
    };

    let builder = ClientBuilder::new().max_concurrent_streams(1);

    let client_fut = async move {
        let (mut conn, mut client) = builder.handshake(io).await.unwrap();
        let request = build_test_request();
        let resp = client.send_request(request).unwrap();
        let _r = conn.drive(resp).await.unwrap();
    };

    join(srv_fut, client_fut).await;
}

#[ignore]
#[tokio::test]
async fn request_over_max_concurrent_streams_errors() {
    support::trace_init!();
    let (io, mut srv) = mock::new();

    let srv_fut = async move {
        let settings = srv
            .assert_client_handshake_with_settings(
                frames::settings()
                    // super tiny server
                    .max_concurrent_streams(1)
                    .initial_window_size(1),
            )
            .await;
        assert_default_settings!(settings);

        srv.recv_frame(frames::headers(1).request(
            "POST",
            "https",
            "example.com",
            "/",
        ))
        .await;
        srv.send_frame(frames::headers(1).response(200).eos())
            .await;
        srv.recv_frame(frames::headers(1).request(
            "POST",
            "https",
            "example.com",
            "/",
        ))
        .await;
        srv.send_frame(frames::headers(3).response(200))
            .await;
        srv.recv_frame(frames::data(3, "hello").eos())
            .await;
        srv.send_frame(frames::data(3, "").eos())
            .await;
        srv.recv_frame(frames::headers(1).request(
            "POST",
            "https",
            "example.com",
            "/",
        ))
        .await;
        srv.send_frame(frames::headers(5).response(200))
            .await;
        srv.recv_frame(frames::data(5, "hello").eos())
            .await;
        srv.send_frame(frames::data(5, "").eos())
            .await;
    };

    let client_fut = async move {
        let (mut conn, mut client) = ClientBuilder::new()
            .handshake(io)
            .await
            .expect("handshake");
        let request = build_test_request_post("www.example.com");
        // first request is allowed
        let resp1_fut = client.send_request(request).unwrap();

        // second request is put into pending_open
        let request = build_test_request_post("www.example.com");
        let resp2_fut = client.send_request(request).unwrap();

        let waker = futures::task::noop_waker();
        let _cx = Context::from_waker(&waker);

        let request = build_test_request_post("www.example.com");
        let err = client.send_request(request);
        if let Err(e) = err {
            dbg!(e);
        }
        conn.drive(async move { resp1_fut.await.unwrap() })
            .await;

        join(async move { conn.await.unwrap() }, async move {
            resp2_fut.await.unwrap()
        })
        .await;
    };

    join(srv_fut, client_fut).await;
}

trait MockH2 {
    fn local_handshake(&mut self) -> &mut Self;
}

impl MockH2 for mock_io::Builder {
    fn local_handshake(&mut self) -> &mut Self {
        self.write(MAGIC_PREFACE)
            // Settings frame
            .write(frames::NEW_SETTINGS)
            .read(frames::NEW_SETTINGS)
            .write(frames::SETTINGS_ACK)
            .read(frames::SETTINGS_ACK)
    }
}
