mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{Room, fake_relay, picking};
use dekan_core::overlay::OverlayTarget;
use dekan_core::state::{StateReceiver, StateSender, new_state_channel};
use dekan_party::client::{PartyClient, PartyExit};
use dekan_party::token::{PartyToken, random_member_id, unix_now};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

struct Party {
    room: Arc<Mutex<Room>>,
    a: (StateSender, StateReceiver),
    b: (StateSender, StateReceiver),
    cancel: CancellationToken,
    tasks: Vec<JoinHandle<PartyExit>>,
}

async fn party_of_two() -> Party {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let relay = format!("ws://{}", listener.local_addr().expect("addr"));
    let room = Arc::new(Mutex::new(Room::default()));
    tokio::spawn(fake_relay(listener, room.clone()));

    let token = PartyToken::generate(random_member_id().expect("rng"), unix_now()).expect("rng");
    let joined = PartyToken::decode(&token.encode(), unix_now()).expect("friend decodes the code");
    let a = new_state_channel();
    a.0.send_replace(picking(103, 103_015, "puuid-a"));
    let b = new_state_channel();
    b.0.send_replace(picking(238, 238_001, "puuid-b"));

    let cancel = CancellationToken::new();
    let mut tasks = Vec::new();
    for (code, (tx, rx)) in [(token, &a), (joined, &b)] {
        let client = PartyClient::new(
            relay.clone(),
            code,
            random_member_id().expect("rng"),
            tx.clone(),
            rx.clone(),
        );
        tasks.push(tokio::spawn(client.run(cancel.clone())));
    }
    Party {
        room,
        a,
        b,
        cancel,
        tasks,
    }
}

async fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    while !done() {
        assert!(tokio::time::Instant::now() < deadline, "timed out: {what}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn sees(rx: &StateReceiver, puuid: &str, entry_id: u32) -> bool {
    let peers = rx.borrow().party_peers.clone();
    peers.len() == 1 && peers[0].puuid == puuid && peers[0].entry_id() == entry_id
}

fn repick(tx: &StateSender, champion_id: u32, skin_id: u32) {
    tx.send_modify(|state| {
        state.overlay_target = Some(OverlayTarget {
            champion_id,
            skin_id,
            chroma_id: None,
        });
    });
}

async fn shut_down(party: Party) {
    party.cancel.cancel();
    for task in party.tasks {
        let _ = task.await; // ignore-ok: test teardown
    }
}

#[tokio::test]
async fn both_members_reconnect_and_see_each_other_after_the_relay_drops_them() {
    let party = party_of_two().await;
    wait_until("first contact", || {
        sees(&party.a.1, "puuid-b", 238_001) && sees(&party.b.1, "puuid-a", 103_015)
    })
    .await;

    std::mem::take(&mut party.room.lock().expect("room lock").kick).cancel();

    wait_until("both reconnected", || {
        party.room.lock().expect("room lock").connections >= 4
    })
    .await;
    wait_until("contact after reconnecting", || {
        sees(&party.a.1, "puuid-b", 238_001) && sees(&party.b.1, "puuid-a", 103_015)
    })
    .await;

    shut_down(party).await;
}

#[tokio::test]
async fn a_lost_member_list_is_made_up_by_the_next_one() {
    let party = party_of_two().await;
    wait_until("first contact", || sees(&party.a.1, "puuid-b", 238_001)).await;

    let announced = party.room.lock().expect("room lock").seen_skins.len();
    party.room.lock().expect("room lock").muted = true;
    repick(&party.b.0, 238, 238_002);
    wait_until("the relay received the new pick", || {
        party.room.lock().expect("room lock").seen_skins.len() > announced
    })
    .await;
    assert!(sees(&party.a.1, "puuid-b", 238_001));

    party.room.lock().expect("room lock").muted = false;
    repick(&party.a.0, 103, 103_016);
    wait_until("the next member list carries the missed pick", || {
        sees(&party.a.1, "puuid-b", 238_002)
    })
    .await;

    shut_down(party).await;
}
