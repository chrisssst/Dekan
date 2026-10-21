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

#[test]
fn test_a_member_joining_again_replaces_its_older_connection() {
    let mut room = Room::new();
    let (old_tx, _old_rx) = tokio::sync::mpsc::channel(8);
    let (new_tx, _new_rx) = tokio::sync::mpsc::channel(8);
    let old = room.add_member(old_tx);
    let new = room.add_member(new_tx);

    room.update_join(old, 42);
    room.update_join(new, 42);

    let snapshot: serde_json::Value = serde_json::from_str(&room.snapshot()).expect("json");
    assert_eq!(snapshot["members"].as_array().map(Vec::len), Some(1));
}

#[test]
fn test_dropped_sessions_do_not_hold_a_seat() {
    let mut room = Room::new();
    let mut receivers = Vec::new();
    for _ in 0..MAX_MEMBERS {
        let (tx, rx) = channel::<String>(MEMBER_QUEUE);
        room.add_member(tx);
        receivers.push(rx);
    }
    assert!(room.is_full());
    receivers.clear();
    assert!(!room.is_full());
    assert!(room.is_empty());
}

#[test]
fn test_a_second_join_of_the_same_member_replaces_the_first() {
    let mut room = Room::new();
    let (first_tx, mut first_rx) = channel::<String>(MEMBER_QUEUE);
    let (second_tx, _second_rx) = channel::<String>(MEMBER_QUEUE);
    let first = room.add_member(first_tx);
    let second = room.add_member(second_tx);
    assert_eq!(room.update_join(first, 7), 0);
    assert_eq!(room.update_join(second, 7), 1);
    while first_rx.try_recv().is_ok() {}
    assert!(matches!(
        first_rx.try_recv(),
        Err(tokio::sync::mpsc::error::TryRecvError::Disconnected)
    ));
}
