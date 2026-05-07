use support::prelude::*;

#[tokio::test]
async fn buffer_limit() {
    support::trace_init!();
    let payload = [0u8; 10];

    let (io, mut srv) = mock::new();

    let client_fut = async move {
        let (mut conn, mut client) = ClientBuilder::new()
            .max_recv_buffer_size(20)
            .handshake(io)
            .await
            .unwrap();
        let request = build_test_request();
        let resp = client.send_request(request).unwrap();
        let mut err = conn.drive(resp).await.unwrap_err();
        assert!(err.err().is_buffer_limit_error());
        let partial = err.take_partial_response().unwrap();
        assert_eq!(partial.body_as_ref().unwrap().len(), 30);
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
        srv.send_frame(frames::data(1, &payload[..]))
            .await;
        srv.send_frame(frames::data(1, &payload[..]))
            .await;
        srv.send_frame(frames::data(1, &payload[..]))
            .await;
    };
    join(srv_fut, client_fut).await;
}
