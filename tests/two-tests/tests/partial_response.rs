use support::prelude::*;
use tokio::time::timeout;

#[tokio::test]
async fn partial_response_headers() {
    support::trace_init!();
    let (io, mut srv) = mock::new();

    let client_fut = async move {
        let (mut conn, mut client) = ClientBuilder::new()
            .handshake(io)
            .await
            .unwrap();

        let request = build_test_request();
        let resp = client.send_request(request).unwrap();
        let resp = conn
            .drive(resp)
            .await
            .unwrap_err()
            .take_partial_response()
            .unwrap();
        assert_eq!(resp.status(), &200);
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

#[tokio::test]
async fn partial_response_take() {
    support::trace_init!();
    let (io, mut srv) = mock::new();

    let client_fut = async move {
        let (mut conn, mut client) = ClientBuilder::new()
            .handshake(io)
            .await
            .unwrap();

        let request = build_test_request();
        let mut resp_fut = client.send_request(request).unwrap();
        let resp_drive = conn.drive(&mut resp_fut);
        let _ =
            timeout(std::time::Duration::from_millis(50), resp_drive).await;
        let resp = resp_fut
            .take_partial_response()
            .unwrap();
        assert_eq!(resp.status(), &200);
        assert!(resp.body_as_ref().is_none());
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
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        srv.send_frame(frames::data(1, vec![0; 16]).eos())
            .await;
    };
    join(srv_fut, client_fut).await;
}
