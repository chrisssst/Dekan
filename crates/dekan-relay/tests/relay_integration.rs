use std::sync::Arc;
use std::time::Duration;

use dekan_core::overlay::OverlayTarget;
use dekan_core::party::PartyStatus;
use dekan_core::phase::GamePhase;
use dekan_core::state::{AppState, new_state_channel};
use dekan_party::client::{PartyClient, PartyExit};
use dekan_party::crypto::room_id;
use dekan_party::token::{PartyToken, random_member_id, unix_now};
use futures_util::StreamExt;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

fn make_state(champion: u32, skin: u32, puuid: &str) -> AppState {
    AppState {
        phase: GamePhase::ChampSelect,
        champion_id: Some(champion),
        local_puuid: Some(puuid.into()),
        overlay_target: Some(OverlayTarget {
            champion_id: champion,
            skin_id: skin,
            chroma_id: None,
        }),
        ..Default::default()
    }
}

#[tokio::test]
async fn test_relay_e2e_two_clients() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral");
    let addr = listener.local_addr().expect("local addr");
    let cancel_server = CancellationToken::new();

    let server = Arc::new(dekan_relay::RelayServer::new());
    let server_handle = tokio::spawn({
        let server = server.clone();
        let cancel = cancel_server.clone();
        async move {
            server.run(listener, cancel).await;
        }
    });

    let relay_url = format!("ws://127.0.0.1:{}", addr.port());

    let resp = reqwest::get(format!("http://127.0.0.1:{}", addr.port()))
        .await
        .expect("get health");
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.expect("health text");
    assert!(body.contains("dekan-party-relay"));

    let token = PartyToken::generate(random_member_id().expect("rng"), unix_now()).expect("rng");
    let joined = PartyToken::decode(&token.encode(), unix_now()).expect("decode token");

    let (tx_a, rx_a) = new_state_channel();
    tx_a.send_replace(make_state(103, 103_015, "puuid-a"));
    let cancel_a = CancellationToken::new();
    let client_a = PartyClient::new(
        relay_url.clone(),
        token.clone(),
        random_member_id().expect("rng"),
        tx_a,
        rx_a.clone(),
    );

    let (tx_b, rx_b) = new_state_channel();
    tx_b.send_replace(make_state(238, 238_001, "puuid-b"));
    let cancel_b = CancellationToken::new();
    let client_b = PartyClient::new(
        relay_url.clone(),
        joined,
        random_member_id().expect("rng"),
        tx_b,
        rx_b.clone(),
    );

    let handle_a = tokio::spawn(client_a.run(cancel_a.clone()));
    let handle_b = tokio::spawn(client_b.run(cancel_b.clone()));

    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let a_sees = rx_a.borrow().party_peers.clone();
        let b_sees = rx_b.borrow().party_peers.clone();
        if a_sees.len() == 1 && b_sees.len() == 1 {
            assert_eq!(a_sees[0].puuid, "puuid-b");
            assert_eq!(a_sees[0].entry_id(), 238_001);
            assert_eq!(b_sees[0].puuid, "puuid-a");
            assert_eq!(b_sees[0].entry_id(), 103_015);
            assert!(matches!(
                rx_a.borrow().party_status,
                PartyStatus::Connected { members: 2 }
            ));
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "peers did not sync: a={a_sees:?} b={b_sees:?}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    cancel_a.cancel();
    cancel_b.cancel();
    let _ = handle_a.await; // ignore-ok: test client task join
    let _ = handle_b.await; // ignore-ok: test client task join

    cancel_server.cancel();
    let _ = server_handle.await; // ignore-ok: test server task join
}

#[tokio::test]
async fn test_a_sixth_member_is_refused_with_409_and_the_client_gives_up() {
    assert_eq!(dekan_relay::MAX_MEMBERS, 5, "one League team per room");

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral");
    let port = listener.local_addr().expect("local addr").port();
    let cancel_server = CancellationToken::new();
    let server = Arc::new(dekan_relay::RelayServer::new());
    let server_handle = tokio::spawn({
        let cancel = cancel_server.clone();
        async move { server.run(listener, cancel).await }
    });

    let relay_url = format!("ws://127.0.0.1:{port}");
    let token = PartyToken::generate(random_member_id().expect("rng"), unix_now()).expect("rng");
    let room_url = format!("{relay_url}/room?key={}", room_id(token.key()));

    let mut members = Vec::new();
    for n in 1..=dekan_relay::MAX_MEMBERS {
        let (mut ws, _) = tokio_tungstenite::connect_async(&room_url)
            .await
            .unwrap_or_else(|e| panic!("member {n} must be let in: {e}"));
        let first = tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("snapshot in time")
            .expect("socket open")
            .expect("snapshot frame");
        assert!(first.is_text(), "member {n} gets the members snapshot");
        members.push(ws);
    }

    match tokio_tungstenite::connect_async(&room_url).await {
        Err(tokio_tungstenite::tungstenite::Error::Http(response)) => {
            assert_eq!(response.status().as_u16(), 409, "a full room answers 409");
        }
        Err(e) => panic!("the sixth member must get 409, got: {e}"),
        Ok(_) => panic!("the sixth member must be refused"),
    }

    let (tx, rx) = new_state_channel();
    tx.send_replace(make_state(103, 103_015, "puuid-6"));
    let client = PartyClient::new(
        relay_url,
        token,
        random_member_id().expect("rng"),
        tx,
        rx.clone(),
    );
    let exit = tokio::time::timeout(
        Duration::from_secs(10),
        client.run(CancellationToken::new()),
    )
    .await
    .expect("a full room ends the client without retrying");
    assert_eq!(exit, PartyExit::RoomFull);
    assert_eq!(rx.borrow().party_status, PartyStatus::Off);

    let mut leaving = members.pop().expect("a member to leave");
    leaving.close(None).await.expect("close handshake");
    drop(leaving);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match tokio_tungstenite::connect_async(&room_url).await {
            Ok(_) => break,
            Err(e) => assert!(
                tokio::time::Instant::now() < deadline,
                "a slot freed by a leaving member must be reusable: {e}"
            ),
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    drop(members);
    cancel_server.cancel();
    let _ = server_handle.await; // ignore-ok: test server task join
}
