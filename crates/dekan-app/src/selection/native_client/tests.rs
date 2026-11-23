use super::*;
use serde_json::json;

#[test]
fn script_payload_preserves_skin_chroma_and_party_shape() {
    use dekan_core::overlay::{CatalogChroma, CatalogSkin};
    let catalog = Catalog {
        champion_id: 238,
        champion_name: "Zed".into(),
        skins: vec![CatalogSkin {
            id: 238001,
            name: "Shockblade Zed".into(),
            name_unknown: false,
            tile: None,
            chromas: vec![CatalogChroma {
                id: 238015,
                name: "Ruby".into(),
                color: Some("#ff0000".into()),
                form: false,
                preview_path: None,
                has_preview: false,
            }],
        }],
        ..Catalog::default()
    };
    let payload = script::snapshot(Some(&catalog));
    assert_eq!(payload["skins"][0]["chromas"][0]["id"], 238015);
    assert!(payload["mods"]["selection"].is_object());
    assert_eq!(payload["party"]["connected_members"], 0);
    assert!(script::snapshot(None).is_null());
}

#[test]
fn commands_are_typed_bounded_and_use_existing_dispatch() {
    let queue = json!([
        {"type":"select","id":238015}, {"type":"clear"},
        {"type":"random"}, {"type":"join_lobby_party"},
        {"type":"open_panel"}, {"type":"open_mods_folder"},
        {"type":"set_identity","loadout":null},
        {"type":"select","id":-1}, {"type":"arbitrary_command"}
    ]);
    let commands = script::commands(&queue);
    assert_eq!(commands.len(), 7);
    assert_eq!(commands[0], OverlayCommand::Select { id: 238015 });
    assert_eq!(commands[6], OverlayCommand::SetIdentity { loadout: None });
    assert!(script::commands(&json!(vec![json!({"type":"clear"}); 65])).is_empty());
}

#[test]
fn embedded_client_uses_dekan_assets_and_repository() {
    let source = script::install().expect("script serialization");
    assert!(source.contains("window.__dekanInClient"));
    assert!(source.contains("https://github.com/chrisssst/Dekan"));
    assert!(source.contains("https://discord.gg/dekan"));
    assert!(source.contains("data:image/png;base64,"));
    assert!(!source.to_ascii_lowercase().contains("bullet"));
    assert!(!source.contains("Isllanrx"));
    assert!(script::sync(&json!(null), None, "tr_TR").contains("__dekanLanguage = \"tr\""));
    assert!(script::sync(&json!(null), None, "fr_FR").contains("__dekanLanguage = \"en\""));
}

#[tokio::test]
async fn cdp_discovers_local_page_matches_ids_and_reports_exceptions() {
    use futures_util::{SinkExt, StreamExt};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio_tungstenite::tungstenite::Message;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    let port = listener.local_addr().expect("address").port();
    let server = async {
        let (mut http, _) = listener.accept().await.expect("HTTP");
        let mut request = [0u8; 4096];
        let count = http.read(&mut request).await.expect("request");
        assert!(String::from_utf8_lossy(&request[..count]).starts_with("GET /json "));
        let body = json!([{"type":"page","url":"https://riot/index.html","webSocketDebuggerUrl":format!("ws://127.0.0.1:{port}/devtools/page/1")}]).to_string();
        http.write_all(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .as_bytes(),
        )
        .await
        .expect("HTTP response");
        drop(http);
        let (stream, _) = listener.accept().await.expect("WebSocket");
        let mut socket = tokio_tungstenite::accept_async(stream)
            .await
            .expect("handshake");
        for fails in [false, true] {
            let text = socket
                .next()
                .await
                .expect("request frame")
                .expect("frame")
                .into_text()
                .expect("text");
            let message: serde_json::Value = serde_json::from_str(&text).expect("JSON");
            assert_eq!(message["method"], "Runtime.evaluate");
            assert_eq!(message["params"]["returnByValue"], true);
            socket
                .send(Message::Text(
                    json!({"id":9999,"result":{}}).to_string().into(),
                ))
                .await
                .expect("unrelated reply");
            let result = if fails {
                json!({"exceptionDetails":{"text":"failure"}})
            } else {
                json!({"result":{"value":{"commands":[],"active":true}}})
            };
            socket
                .send(Message::Text(
                    json!({"id":message["id"],"result":result})
                        .to_string()
                        .into(),
                ))
                .await
                .expect("evaluation reply");
        }
    };
    let client = async {
        let mut connection = cdp::Connection::discover(port).await.expect("discover");
        assert_eq!(
            connection.evaluate("one").await.expect("evaluate")["active"],
            true
        );
        assert!(connection.evaluate("two").await.is_err());
    };
    tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(server, client);
    })
    .await
    .expect("bounded mock exchange");
}
