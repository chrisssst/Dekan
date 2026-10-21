use std::sync::{Arc, Mutex};

use dekan_core::overlay::OverlayTarget;
use dekan_core::phase::GamePhase;
use dekan_core::state::AppState;
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;

#[derive(Default)]
pub struct Room {
    members: Vec<(usize, UnboundedSender<String>, serde_json::Value)>,
    pub seen_skins: Vec<serde_json::Value>,
    pub muted: bool,
    pub connections: usize,
    pub kick: CancellationToken,
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
    if room.muted {
        return;
    }
    let payload = snapshot(room);
    for (_, tx, _) in &room.members {
        let _ = tx.send(payload.clone()); // ignore-ok: a closed test socket just misses the update
    }
}

pub async fn fake_relay(listener: TcpListener, room: Arc<Mutex<Room>>) {
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
            let kick = {
                let mut guard = room.lock().expect("room lock");
                guard.connections += 1;
                guard.members.push((id, tx.clone(), serde_json::json!({})));
                let _ = tx.send(snapshot(&guard)); // ignore-ok: test relay
                guard.kick.clone()
            };
            loop {
                tokio::select! {
                    () = kick.cancelled() => break,
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

pub fn picking(champion: u32, skin: u32, puuid: &str) -> AppState {
    AppState {
        phase: GamePhase::ChampSelect,
        champion_id: Some(champion),
        local_puuid: Some(puuid.into()),
        party_share: true,
        overlay_target: Some(OverlayTarget {
            champion_id: champion,
            skin_id: skin,
            chroma_id: None,
        }),
        ..Default::default()
    }
}
