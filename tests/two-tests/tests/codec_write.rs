use support::prelude::*;

#[tokio::test]
async fn write_continuation_frames() {
    // An invalid dependency ID results in a stream level error. The hpack
    // payload should still be decoded.
    support::trace_init!();
    let (io, mut srv) = mock::new();

    let large = build_large_headers();

    // Build the large request frame
    let frame = large.iter().fold(
        frames::headers(1).request("GET", "https", "http2.akamai.com", "/"),
        |frame, &(name, ref value)| frame.field(name, &value[..]),
    );

    let srv = async move {
        let settings = srv.assert_client_handshake().await;
        assert_default_settings!(settings);
        srv.recv_frame(frame.eos()).await;
        srv.send_frame(frames::headers(1).response(204).eos())
            .await;
    };

    let client = async move {
        let (mut conn, mut client) = ClientBuilder::new()
            .handshake(io)
            .await
            .expect("handshake");

        let mut request = build_test_request();

        let mut headermap = HeaderMap::new();
        for &(name, ref value) in &large {
            let name: http::HeaderName = name.try_into().unwrap();
            let value = value.try_into().unwrap();
            headermap
                .try_append(name, value)
                .unwrap();
        }

        request.set_headers(headermap);

        let req = async {
            let res = client
                .send_request(request)
                .expect("send_request1")
                .await;
            let response = res.unwrap();
            assert_eq!(response.status(), StatusCode::NO_CONTENT);
        };

        conn.drive(req).await;
        conn.await.unwrap();
    };

    join(srv, client).await;
}

#[tokio::test]
async fn client_settings_header_table_size() {
    // A server sets the SETTINGS_HEADER_TABLE_SIZE to 0, test that the
    // client doesn't send indexed headers.
    support::trace_init!();

    let io = mock_io::Builder::new()
        // Read SETTINGS_HEADER_TABLE_SIZE = 0
        .handshake_read_settings(&[
            0, 0, 6, // len
            4, // type
            0, // flags
            0, 0, 0, 0, // stream id
            0, 0x1, // id = SETTINGS_HEADER_TABLE_SIZE
            0, 0, 0, 0, // value = 0
        ])
        // Write GET / (1st)
        .write(&[
            0, 0, 0x11, 1, 5, 0, 0, 0, 1,    // 17 bytes, stream 1
            0x20, // Dynamic table size update to 0 (ONLY in first request)
            0x82, 0x87,
            0x01, // :method GET, :scheme https, :authority (literal)
            0x8B, 0x9D, 0x29, 0xAC, 0x4B, 0x8F, 0xA8, 0xE9, 0x19, 0x97, 0x21,
            0xE9, 0x84, // :path /
        ])
        //// Read response
        .read(&[0, 0, 1, 1, 5, 0, 0, 0, 1, 137])
        //// Write GET / (2nd, doesn't use indexed headers)
        //// - Sends 0x20 about size change
        //// - Sends :authority as literal instead of indexed
        .write(&[
            0, 0, 0x10, 1, 5, 0, 0, 0, 3, // 16 bytes (no 0x20), stream 3
            0x82, 0x87,
            0x01, // :method GET, :scheme https, :authority (literal)
            0x8B, 0x9D, 0x29, 0xAC, 0x4B, 0x8F, 0xA8, 0xE9, 0x19, 0x97, 0x21,
            0xE9, 0x84, // :path /
        ])
        .read(&[0, 0, 1, 1, 5, 0, 0, 0, 3, 137])
        .build();

    let (mut conn, mut client) = ClientBuilder::new()
        .handshake(io)
        .await
        .expect("handshake");

    let request = build_test_request();
    let req1 = client.send_request(request).unwrap();
    conn.drive(req1).await.expect("req1");

    let request = build_test_request();
    let req2 = client.send_request(request).unwrap();
    conn.drive(req2).await.expect("req2");
}

#[tokio::test]
async fn server_settings_header_table_size() {
    // A client sets the SETTINGS_HEADER_TABLE_SIZE to 0, test that the
    // server doesn't send indexed headers.
    support::trace_init!();

    let io = mock_io::Builder::new()
        .read(MAGIC_PREFACE)
        // Read SETTINGS_HEADER_TABLE_SIZE = 0
        .read(&[
            0, 0, 6, // len
            4, // type
            0, // flags
            0, 0, 0, 0, // stream id
            0, 0x1, // id = SETTINGS_HEADER_TABLE_SIZE
            0, 0, 0, 0, // value = 0
        ])
        .write(frames::NEW_SETTINGS)
        .write(frames::SETTINGS_ACK)
        .read(frames::SETTINGS_ACK)
        // Write GET /
        .read(&[
            0, 0, 0x10, 1, 5, 0, 0, 0, 1, 0x82, 0x87, 0x41, 0x8B, 0x9D, 0x29,
            0xAC, 0x4B, 0x8F, 0xA8, 0xE9, 0x19, 0x97, 0x21, 0xE9, 0x84,
        ])
        // Read response
        .write(&[0, 0, 7, 1, 5, 0, 0, 0, 1, 32, 136, 0, 129, 31, 129, 143])
        .build();

    let mut srv = ServerBuilder::new()
        .handshake(io)
        .await
        .expect("handshake");

    let (_req, mut stream) = srv.accept().await.unwrap().unwrap();

    let scode = ResponseLine {
        status: StatusCode::from_u16(200).unwrap(),
    };

    let mut headers = HeaderMap::new();
    headers.insert("a", "b".parse().unwrap());
    let res = Response::new(scode, headers, None, None);
    stream.send_response(res).unwrap();
    assert!(srv.accept().await.is_none());
}
