use super::*;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::handshake::server::Request;

#[test]
fn recovery_is_scoped_to_network_failures_on_the_default_relay() {
    let error = || Error::Io(io::Error::from(io::ErrorKind::ConnectionRefused));
    let default = format!(
        "{}/room?key=private-room",
        super::super::config::DEFAULT_RELAY_URL
    );
    assert!(should_recover(&default, &error()));
    for url in [
        "ws://dekan-party-relay.heeazk.workers.dev/room",
        "wss://dekan-party-relay.heeazk.workers.dev:444/room",
        "wss://dekan-party-relay.heeazk.workers.dev.attacker.example/room",
        "wss://custom.example/room",
        "ws://127.0.0.1:8787/room",
        "invalid",
    ] {
        assert!(!should_recover(url, &error()), "{url}");
    }
    let full = http::Response::builder()
        .status(409)
        .body(None)
        .expect("response");
    assert!(!should_recover(&default, &Error::Http(Box::new(full))));
    assert!(!should_recover(&default, &Error::ConnectionClosed));
}

#[tokio::test]
#[allow(
    clippy::result_large_err,
    reason = "The upstream WebSocket callback requires this error response type"
)]
async fn alternate_address_preserves_the_original_host_and_room_path() {
    let refused = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("refused address");
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let refused_address = refused.local_addr().expect("address");
    let address = listener.local_addr().expect("address");
    drop(refused);
    let server = async {
        let (socket, _) = listener.accept().await.expect("accept");
        let mut ws = tokio_tungstenite::accept_hdr_async(socket, |request: &Request, response| {
            assert_eq!(request.headers()["host"], "relay.example");
            assert_eq!(request.uri(), "/room?key=original-room");
            Ok(response)
        })
        .await
        .expect("handshake");
        ws.close(None).await.expect("server close");
    };
    let client = async {
        let (mut ws, _) = connect_addresses(
            "ws://relay.example/room?key=original-room",
            WebSocketConfig::default(),
            &[refused_address, address],
        )
        .await
        .expect("second address connects");
        ws.close(None).await.expect("client close");
    };
    tokio::time::timeout(Duration::from_secs(3), async {
        tokio::join!(server, client)
    })
    .await
    .expect("bounded test");
}

#[tokio::test]
#[allow(
    clippy::result_large_err,
    reason = "The upstream WebSocket callback requires this error response type"
)]
async fn room_full_response_stops_address_failover() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let unused = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("unused listener");
    let addresses = [
        listener.local_addr().expect("address"),
        unused.local_addr().expect("address"),
    ];
    let server = async {
        let (socket, _) = listener.accept().await.expect("accept");
        let result = tokio_tungstenite::accept_hdr_async(socket, |_: &Request, _| {
            Err(http::Response::builder()
                .status(409)
                .body(Some("room full".into()))
                .expect("response"))
        })
        .await;
        assert!(matches!(result, Err(Error::Http(_))));
    };
    let client = async {
        let result = connect_addresses(
            "ws://relay.example/room",
            WebSocketConfig::default(),
            &addresses,
        )
        .await;
        assert!(matches!(result, Err(Error::Http(response)) if response.status().as_u16() == 409));
    };
    tokio::join!(server, client);
    assert!(
        tokio::time::timeout(Duration::from_millis(20), unused.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn an_incomplete_handshake_can_be_cancelled_without_a_detached_task() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let url = format!("ws://{}/room", listener.local_addr().expect("address"));
    let client = connect(&url, WebSocketConfig::default());
    assert!(
        tokio::time::timeout(Duration::from_millis(20), client)
            .await
            .is_err()
    );
}
