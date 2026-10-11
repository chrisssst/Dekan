use super::*;

#[tokio::test]
async fn test_failed_handshakes_leave_no_room_and_oversized_frames_close_the_session() {
    use tokio::io::AsyncWriteExt;

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let server = Arc::new(RelayServer::new());
    let cancel = CancellationToken::new();
    let running = tokio::spawn(server.clone().run(listener, cancel.clone()));
    let key = "0123456789abcdef0123456789abcdef";

    for _ in 0..50 {
        let mut raw = TcpStream::connect(addr).await.expect("connect");
        raw.write_all(
            format!("GET /room?key={key} HTTP/1.1\r\nUpgrade: websocket\r\n\r\n").as_bytes(),
        )
        .await
        .expect("write");
        drop(raw);
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        server.rooms.read().await.is_empty(),
        "a failed handshake created a room"
    );

    let url = format!("ws://{addr}/room?key={key}");
    let (mut ws, _) = tokio_tungstenite::connect_async(url).await.expect("join");
    let first = ws.next().await.expect("snapshot").expect("frame");
    assert!(first.to_text().expect("text").contains("members"));
    assert_eq!(server.rooms.read().await.len(), 1);
    let oversized = "x".repeat(MAX_MESSAGE_BYTES * 4);
    let _ = ws.send(Message::Text(oversized.into())).await; // ignore-ok: the server may already be closing
    let closed = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match ws.next().await {
                None | Some(Err(_)) | Some(Ok(Message::Close(_))) => break,
                Some(Ok(_)) => {}
            }
        }
    })
    .await;
    assert!(closed.is_ok(), "an oversized frame must end the session");
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        server.rooms.read().await.is_empty(),
        "the emptied room is removed"
    );

    cancel.cancel();
    let _ = running.await; // ignore-ok: the server loop ends with the token
}

#[test]
fn test_valid_room_key() {
    assert!(is_valid_room_key("0123456789abcdef0123456789abcdef"));
    assert!(!is_valid_room_key("short"));
    assert!(!is_valid_room_key("0123456789ABCDEF0123456789ABCDEF"));
    assert!(!is_valid_room_key("0123456789abcdef0123456789abcdefg"));
}

#[test]
fn test_extract_room_key() {
    let req = "GET /room?key=0123456789abcdef0123456789abcdef HTTP/1.1\r\nHost: localhost\r\n";
    assert_eq!(
        RelayServer::extract_room_key(req),
        Some("0123456789abcdef0123456789abcdef".to_string())
    );

    let bad_req = "GET /room?key=badkey HTTP/1.1\r\n";
    assert_eq!(RelayServer::extract_room_key(bad_req), None);
}
