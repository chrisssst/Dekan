use std::sync::{Arc, Mutex};
use std::time::Duration;

use dekan_core::overlay::OverlayTarget;
use dekan_core::party::PartyStatus;
use dekan_core::phase::GamePhase;
use dekan_core::state::{AppState, new_state_channel};
use dekan_party::client::PartyClient;
use dekan_party::token::{PartyToken, random_member_id, unix_now};
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;

#[derive(Default)]
struct Room {
    members: Vec<(usize, UnboundedSender<String>, serde_json::Value)>,

    seen_skins: Vec<serde_json::Value>,
}

fn snapshot(room: &Room) -> String {
    let members: Vec<serde_json::Value> = room
        .members
        .iter()
        .filter(|(_, _, info)| info.get("summoner_id").is_some())
        .map(|(_, _, info)| info.clone())
        .collect();
    serde_json::json!({ "type": "members", "members": members }).to_string()
}

fn broadcast(room: &Room) {
    let payload = snapshot(room);
    for (_, tx, _) in &room.members {
        let _ = tx.send(payload.clone()); // ignore-ok: a closed test socket just misses the update
    }
}

async fn fake_relay(listener: TcpListener, room: Arc<Mutex<Room>>) {
    let mut next_id = 0usize;
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            return;
        };
        next_id += 1;
        let id = next_id;
        let room = room.clone();
        tokio::spawn(async move {
            let Ok(ws) = tokio_tungstenite::accept_async(stream).await else {
                return;
            };
            let (mut write, mut read) = ws.split();
            let (tx, mut rx) = unbounded_channel::<String>();
            {
                let mut guard = room.lock().expect("room lock");
                guard.members.push((id, tx.clone(), serde_json::json!({})));
                let _ = tx.send(snapshot(&guard)); // ignore-ok: test relay
            }
            loop {
                tokio::select! {
                    out = rx.recv() => {
                        let Some(out) = out else { break };
                        if write.send(Message::Text(out.into())).await.is_err() {
                            break;
                        }
                    }
                    incoming = read.next() => {
                        let Some(Ok(Message::Text(text))) = incoming else { break };
                        if text == "ping" {
                            let _ = tx.send("pong".into()); // ignore-ok: test relay
                            continue;
                        }
                        let Ok(msg) = serde_json::from_str::<serde_json::Value>(&text) else {
                            continue;
                        };
                        let mut guard = room.lock().expect("room lock");
                        match msg["type"].as_str() {
                            Some("join") => {
                                if let Some(entry) = guard.members.iter_mut().find(|m| m.0 == id) {
                                    entry.2 = serde_json::json!({
                                        "summoner_id": msg["summoner_id"],
                                        "summoner_name": "Unknown",
                                    });
                                }
                                broadcast(&guard);
                            }
                            Some("skin") => {
                                guard.seen_skins.push(msg["skin"].clone());
                                if let Some(entry) = guard.members.iter_mut().find(|m| m.0 == id) {
                                    entry.2["skin"] = msg["skin"].clone();
                                }
                                broadcast(&guard);
                            }
                            _ => {}
                        }
                    }
                }
            }
            let mut guard = room.lock().expect("room lock");
            guard.members.retain(|m| m.0 != id);
            broadcast(&guard);
        });
    }
}

fn picking(champion: u32, skin: u32, puuid: &str) -> AppState {
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
