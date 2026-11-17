use super::*;

#[test]
fn pack_mod_fantome_creates_zip_with_stored_wads_and_deflated_meta() {
    let dir = std::env::temp_dir().join(format!("dekan_test_pack_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: cleanup before test
    let mod_dir = dir.join("mod_vayne");
    let meta_dir = mod_dir.join("META");
    let wad_dir = mod_dir.join("WAD");
    std::fs::create_dir_all(&meta_dir).expect("meta dir");
    std::fs::create_dir_all(&wad_dir).expect("wad dir");
    std::fs::write(meta_dir.join("info.json"), br#"{"name":"Vayne"}"#).expect("info");
    std::fs::write(wad_dir.join("Vayne.wad.client"), b"wad bytes").expect("wad");

    let staging_file = dir.join("staging").join("mod_vayne.fantome");
    pack_mod_fantome(&mod_dir, &staging_file).expect("pack fantome");

    assert!(staging_file.is_file());
    let file = std::fs::File::open(&staging_file).expect("open zip");
    let mut archive = zip::ZipArchive::new(file).expect("read archive");

    let meta_compression = archive
        .by_name("META/info.json")
        .expect("meta entry")
        .compression();
    assert_eq!(meta_compression, zip::CompressionMethod::Deflated);

    let wad_compression = archive
        .by_name("WAD/Vayne.wad.client")
        .expect("wad entry")
        .compression();
    assert_eq!(wad_compression, zip::CompressionMethod::Stored);

    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: cleanup after test
}

#[tokio::test]
async fn start_patcher_with_retry_waits_for_stop_on_already_running() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let start_calls = Arc::new(AtomicUsize::new(0));
    let running = Arc::new(AtomicBool::new(true));

    let sc = Arc::clone(&start_calls);
    let rn = Arc::clone(&running);
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let sc = Arc::clone(&sc);
            let rn = Arc::clone(&rn);
            tokio::spawn(async move {
                let (r, mut w) = stream.into_split();
                let mut reader = BufReader::new(r);
                let mut line = String::new();
                if reader.read_line(&mut line).await.unwrap_or(0) > 0 {
                    let req: serde_json::Value = serde_json::from_str(&line).expect("json");
                    let cmd = req["cmd"].as_str().unwrap_or("");
                    let resp = match cmd {
                        "start_patcher" => {
                            if sc.fetch_add(1, Ordering::SeqCst) == 0 {
                                serde_json::json!({
                                    "ok": false,
                                    "response": {
                                        "ok": false,
                                        "error": { "kind": "Patcher", "error": "AlreadyRunning" }
                                    }
                                })
                            } else {
                                rn.store(true, Ordering::SeqCst);
                                serde_json::json!({ "ok": true })
                            }
                        }
                        "stop_patcher" => {
                            rn.store(false, Ordering::SeqCst);
                            serde_json::json!({ "ok": true })
                        }
                        "patcher_status" => {
                            let is_run = rn.load(Ordering::SeqCst);
                            serde_json::json!({
                                "ok": true,
                                "response": { "ok": true, "value": { "running": is_run, "phase": "idle" } }
                            })
                        }
                        _ => serde_json::json!({ "ok": true }),
                    };
                    let mut bytes = serde_json::to_vec(&resp).expect("ser");
                    bytes.push(b'\n');
                    let _ = w.write_all(&bytes).await; // ignore-ok: test mock response
                }
            });
        }
    });

    let client = BridgeClient::new(crate::ltf::BridgeEndpoint {
        port,
        token: "tok".to_string(),
        pid: Some(1),
    });
    start_patcher_with_retry(&client)
        .await
        .expect("retry succeeds");
    assert_eq!(start_calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn disable_and_restore_other_enabled_mods_only_touches_non_session_mods() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let toggled = Arc::new(Mutex::new(Vec::<(String, bool)>::new()));
    let toggled_srv = Arc::clone(&toggled);

    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let tog = Arc::clone(&toggled_srv);
            tokio::spawn(async move {
                let (r, mut w) = stream.into_split();
                let mut reader = BufReader::new(r);
                let mut line = String::new();
                if reader.read_line(&mut line).await.unwrap_or(0) > 0 {
                    let req: serde_json::Value = serde_json::from_str(&line).expect("json");
                    let cmd = req["cmd"].as_str().unwrap_or("");
                    let resp = match cmd {
                        "list" => serde_json::json!({
                            "ok": true,
                            "library": [
                                { "id": "session-mod", "name": "kaisa", "enabled": true },
                                { "id": "user-mod-1", "name": "map", "enabled": true },
                                { "id": "user-mod-2", "name": "hud", "enabled": false }
                            ]
                        }),
                        "toggle" => {
                            let target = req["target"].as_str().unwrap_or("").to_string();
                            let enabled = req["enabled"].as_bool().unwrap_or(false);
                            tog.lock().await.push((target, enabled));
                            serde_json::json!({ "ok": true })
                        }
                        _ => serde_json::json!({ "ok": true }),
                    };
                    let mut bytes = serde_json::to_vec(&resp).expect("ser");
                    bytes.push(b'\n');
                    let _ = w.write_all(&bytes).await; // ignore-ok: test mock response
                }
            });
        }
    });

    let client = BridgeClient::new(crate::ltf::BridgeEndpoint {
        port,
        token: "tok".to_string(),
        pid: Some(1),
    });
    let disabled = disable_other_enabled_mods(&client, &["session-mod".to_string()]).await;
    assert_eq!(disabled, vec!["user-mod-1".to_string()]);
    super::ops::restore_disabled_mods(&client, disabled).await;
    let recorded = toggled.lock().await.clone();
    assert_eq!(
        recorded,
        vec![
            ("user-mod-1".to_string(), false),
            ("user-mod-1".to_string(), true)
        ]
    );
}
