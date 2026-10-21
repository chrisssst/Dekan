mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{Room, fake_relay, picking};
use dekan_core::party::PartyStatus;
use dekan_core::state::new_state_channel;
use dekan_party::client::PartyClient;
use dekan_party::token::{PartyToken, random_member_id, unix_now};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn two_clients_see_each_others_pick_through_an_opaque_relay() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let relay = format!("ws://{}", listener.local_addr().expect("addr"));
    let room = Arc::new(Mutex::new(Room::default()));
    tokio::spawn(fake_relay(listener, room.clone()));

    let token = PartyToken::generate(random_member_id().expect("rng"), unix_now()).expect("rng");
    let joined = PartyToken::decode(&token.encode(), unix_now()).expect("friend decodes the code");

    let (a_tx, a_rx) = new_state_channel();
    a_tx.send_replace(picking(103, 103_015, "puuid-a"));
    let (b_tx, b_rx) = new_state_channel();
    b_tx.send_replace(picking(238, 238_001, "puuid-b"));

    let cancel = CancellationToken::new();
    let a = PartyClient::new(
        relay.clone(),
        token,
        random_member_id().expect("rng"),
        a_tx,
        a_rx.clone(),
    );
    let b = PartyClient::new(
        relay,
        joined,
        random_member_id().expect("rng"),
        b_tx,
        b_rx.clone(),
    );
    let a_task = tokio::spawn(a.run(cancel.clone()));
    let b_task = tokio::spawn(b.run(cancel.clone()));

    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let a_sees = a_rx.borrow().party_peers.clone();
        let b_sees = b_rx.borrow().party_peers.clone();
        if a_sees.len() == 1 && b_sees.len() == 1 {
            assert_eq!(a_sees[0].puuid, "puuid-b");
            assert_eq!(a_sees[0].entry_id(), 238_001);
            assert_eq!(b_sees[0].puuid, "puuid-a");
            assert_eq!(b_sees[0].entry_id(), 103_015);
            assert!(matches!(
                a_rx.borrow().party_status,
                PartyStatus::Connected { members: 2 }
            ));
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "peers never appeared: a={a_sees:?} b={b_sees:?}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let seen = room.lock().expect("room lock").seen_skins.clone();
    assert!(!seen.is_empty());
    for skin in &seen {
        let text = skin.to_string();
        assert!(skin.get("c").is_some() && skin.get("n").is_some(), "{text}");
        assert!(!text.contains("puuid") && !text.contains("103015") && !text.contains("238001"));
    }

    cancel.cancel();
    let _ = a_task.await; // ignore-ok: test teardown
    let _ = b_task.await; // ignore-ok: test teardown
    assert_eq!(a_rx.borrow().party_status, PartyStatus::Off);
    assert!(a_rx.borrow().party_peers.is_empty());
}

#[tokio::test]
async fn friends_in_one_lobby_meet_without_a_code_and_share_only_after_agreeing() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let relay = format!("ws://{}", listener.local_addr().expect("addr"));
    let room = Arc::new(Mutex::new(Room::default()));
    tokio::spawn(fake_relay(listener, room.clone()));

    let waiting = |champion, skin, puuid| dekan_core::state::AppState {
        party_share: false,
        ..picking(champion, skin, puuid)
    };
    let (a_tx, a_rx) = new_state_channel();
    a_tx.send_replace(waiting(103, 103_015, "puuid-a"));
    let (b_tx, b_rx) = new_state_channel();
    b_tx.send_replace(waiting(238, 238_001, "puuid-b"));

    let cancel = CancellationToken::new();
    let mut tasks = Vec::new();
    for (tx, rx) in [(&a_tx, &a_rx), (&b_tx, &b_rx)] {
        let client = PartyClient::new(
            relay.clone(),
            PartyToken::for_lobby("6f1c2a0e-party"),
            random_member_id().expect("rng"),
            tx.clone(),
            rx.clone(),
        );
        tasks.push(tokio::spawn(client.run(cancel.clone())));
    }

    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while !matches!(
        b_rx.borrow().party_status,
        PartyStatus::Connected { members: 2 }
    ) {
        assert!(
            tokio::time::Instant::now() < deadline,
            "never met in the lobby room"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(a_rx.borrow().party_peers.is_empty());
    assert!(b_rx.borrow().party_peers.is_empty());
    assert!(
        room.lock()
            .expect("room lock")
            .seen_skins
            .iter()
            .all(serde_json::Value::is_null),
        "nothing is announced before agreeing"
    );

    dekan_core::state::set_party_share(&a_tx, true);
    dekan_core::state::set_party_share(&b_tx, true);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let a_sees = a_rx.borrow().party_peers.clone();
        let b_sees = b_rx.borrow().party_peers.clone();
        if a_sees.len() == 1 && b_sees.len() == 1 {
            assert_eq!(a_sees[0].entry_id(), 238_001);
            assert_eq!(b_sees[0].entry_id(), 103_015);
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "picks never arrived"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    cancel.cancel();
    for task in tasks {
        let _ = task.await; // ignore-ok: test teardown
    }
}
