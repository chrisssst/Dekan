use super::*;

async fn hello_server(response: serde_json::Value) -> (u16, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("address").port();
    let task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept");
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);
        let mut line = String::new();
        reader.read_line(&mut line).await.expect("hello request");
        let request: serde_json::Value = serde_json::from_str(&line).expect("request JSON");
        assert_eq!(request["cmd"], "hello");
        let mut bytes = serde_json::to_vec(&response).expect("response JSON");
        bytes.push(b'\n');
        writer.write_all(&bytes).await.expect("response");
    });
    (port, task)
}

#[tokio::test]
async fn supplied_bridge_identity_is_accepted_with_its_manager_pid() {
    let (port, task) = hello_server(serde_json::json!({
        "ok": true, "service": "BulletBridge", "pid": 42,
        "desktop": "WinSta0\\Default", "auth": true
    }))
    .await;
    assert_eq!(
        probe_port(port, Duration::from_secs(1)).await,
        Some((port, Some(42)))
    );
    task.await.expect("server");
}

#[tokio::test]
async fn obsolete_bridge_and_unrelated_services_are_not_reused() {
    for service in ["DekanBridge", "unrelated-service"] {
        let (port, task) = hello_server(serde_json::json!({
            "ok": true, "service": service, "pid": 42
        }))
        .await;
        assert!(probe_port(port, Duration::from_secs(1)).await.is_none());
        task.await.expect("server");
    }
}

#[tokio::test]
async fn a_failed_hello_is_not_a_live_bridge() {
    let (port, task) = hello_server(serde_json::json!({
        "ok": false, "service": "BulletBridge", "pid": 42
    }))
    .await;
    assert!(probe_port(port, Duration::from_secs(1)).await.is_none());
    task.await.expect("server");
}
