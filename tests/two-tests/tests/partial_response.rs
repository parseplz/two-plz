use support::prelude::*;

#[ignore]
#[tokio::test]
async fn partial_response_headers() {
    //support::trace_init!();
    let (io, mut srv) = mock::new();

    let client_fut = async move {
        let (mut conn, mut client) = ClientBuilder::new()
            .handshake(io)
            .await
            .unwrap();

        let request = build_test_request();
        let resp = client.send_request(request).unwrap();
        let resp = conn.drive(resp).await.unwrap();
        dbg!(resp);
        conn.await.expect("client");
    };

    let srv_fut = async move {
        let settings = srv.assert_client_handshake().await;
        assert_default_settings!(settings);
        srv.recv_frame(
            frames::headers(1)
                .request("GET", "https", "http2.akamai.com", "/")
                .eos(),
        )
        .await;
        srv.send_frame(frames::headers(1).response(200))
            .await;
        srv.send_frame(frames::reset(1).stream_closed())
            .await;
    };
    join(srv_fut, client_fut).await;
}
