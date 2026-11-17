use super::*;

#[test]
fn extract_imported_mod_id_reads_nested_response_value_id() {
    let resp = serde_json::json!({
        "ok": true,
        "response": {
            "ok": true,
            "value": {
                "id": "uuid-mod-123",
                "name": "std_vayne_54",
                "enabled": false
            }
        }
    });
    assert_eq!(extract_imported_mod_id(&resp), "uuid-mod-123");

    let manager_133_resp = serde_json::json!({
        "ok": true,
        "response": {
            "ok": true,
            "value": {
                "kind": "installed",
                "mod": {
                    "id": "28c798ff-3c2b-4505-8236-3ff328e318f9",
                    "name": "vayne-skin-54",
                    "enabled": true
                }
            }
        }
    });
    assert_eq!(
        extract_imported_mod_id(&manager_133_resp),
        "28c798ff-3c2b-4505-8236-3ff328e318f9"
    );
}

#[test]
fn extract_patcher_state_reads_nested_tauri_status() {
    let resp = serde_json::json!({
        "ok": true,
        "response": {
            "ok": true,
            "value": {
                "running": true,
                "phase": "patching"
            }
        }
    });
    let (running, attached, phase) = extract_patcher_state(&resp);
    assert!(running);
    assert!(!attached);
    assert_eq!(phase.as_deref(), Some("patching"));

    let injected = serde_json::json!({
        "ok": true,
        "response": {
            "ok": true,
            "value": {
                "running": true,
                "phase": "injected"
            }
        }
    });
    let (inj_run, inj_att, inj_phase) = extract_patcher_state(&injected);
    assert!(inj_run);
    assert!(inj_att);
    assert_eq!(inj_phase.as_deref(), Some("injected"));

    let building = serde_json::json!({
        "ok": true,
        "response": {
            "ok": true,
            "value": {
                "running": true,
                "phase": "building"
            }
        }
    });
    let (b_run, b_att, _) = extract_patcher_state(&building);
    assert!(b_run);
    assert!(!b_att);
}

#[test]
fn extract_patcher_state_idle_is_not_running() {
    let resp = serde_json::json!({
        "ok": true,
        "response": {
            "ok": true,
            "value": {
                "running": false,
                "phase": "idle"
            }
        }
    });
    let (running, attached, phase) = extract_patcher_state(&resp);
    assert!(!running);
    assert!(!attached);
    assert_eq!(phase.as_deref(), Some("idle"));
}

#[tokio::test]
async fn tcp_bridge_full_cycle_validates_token_and_mod_id() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let expected_token = "deadbeefdeadbeefdeadbeefdeadbeef".to_string();
    let server_token = expected_token.clone();

    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let tok = server_token.clone();
            tokio::spawn(async move {
                let (r, mut w) = stream.into_split();
                let mut reader = BufReader::new(r);
                let mut line = String::new();
                if reader.read_line(&mut line).await.unwrap_or(0) > 0 {
                    let req: serde_json::Value = serde_json::from_str(&line).expect("json");
                    let cmd = req["cmd"].as_str().unwrap_or("");
                    let resp = if cmd == "ping" {
                        serde_json::json!({"ok": true, "pong": 1})
                    } else if req["token"].as_str() != Some(tok.as_str()) {
                        serde_json::json!({"ok": false, "error": "unauthorized"})
                    } else if cmd == "import" {
                        serde_json::json!({"ok": true, "response": {"ok": true, "value": {"mod": {"id": "imported-id-99"}}}})
                    } else if cmd == "patcher_status" {
                        serde_json::json!({"ok": true, "response": {"ok": true, "value": {"running": true, "phase": "injected"}}})
                    } else {
                        serde_json::json!({"ok": true})
                    };
                    let mut bytes = serde_json::to_vec(&resp).expect("ser");
                    bytes.push(b'\n');
                    let _ = w.write_all(&bytes).await; // ignore-ok: test mock socket write
                }
            });
        }
    });

    let endpoint = BridgeEndpoint {
        port,
        token: expected_token,
        pid: Some(9999),
    };
    let client = BridgeClient::new(endpoint);
    client.ping().await.expect("ping");
    let id = client
        .import_archive(Path::new("skin.fantome"), Some("skin"))
        .await
        .expect("import");
    assert_eq!(id, "imported-id-99");
    client.toggle_mod(&id, true).await.expect("toggle");
    client.start_patcher().await.expect("start_patcher");
    let status = client.patcher_status().await.expect("status");
    let (running, attached, _) = extract_patcher_state(&status);
    assert!(running && attached);
}

#[tokio::test]
async fn structured_tauri_error_object_is_preserved_in_error_message() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let expected_token = "tok".to_string();

    tokio::spawn(async move {
        if let Ok((stream, _)) = listener.accept().await {
            let (r, mut w) = stream.into_split();
            let mut reader = BufReader::new(r);
            let mut line = String::new();
            let _ = reader.read_line(&mut line).await; // ignore-ok: test mock read
            let resp = serde_json::json!({
                "ok": false,
                "response": {
                    "ok": false,
                    "error": { "kind": "Patcher", "error": "AlreadyRunning" }
                }
            });
            let mut bytes = serde_json::to_vec(&resp).expect("ser");
            bytes.push(b'\n');
            let _ = w.write_all(&bytes).await; // ignore-ok: test mock write
        }
    });

    let client = BridgeClient::new(BridgeEndpoint {
        port,
        token: expected_token,
        pid: Some(1),
    });
    let err = client
        .start_patcher()
        .await
        .expect_err("expected AlreadyRunning error");
    let msg = err.to_string();
    assert!(
        msg.contains("AlreadyRunning"),
        "expected AlreadyRunning in error message, got: {msg}"
    );
}
