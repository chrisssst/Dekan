use super::*;
use crate::crypto::{RoomCipher, room_id};
use crate::protocol::{Announcement, ClientMessage, RelayMember, RelayMessage, open_member};
use crate::token::PartyToken;
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

async fn receive(socket: &mut Socket, with_skin: bool) -> Vec<RelayMember> {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let message = socket.next().await.expect("relay open").expect("message");
            let Message::Text(text) = message else {
                continue;
            };
            let Ok(RelayMessage::Members { members }) = serde_json::from_str(&text) else {
                continue;
            };
            if members.len() == 2
                && (!with_skin || members.iter().any(|member| member.skin.is_some()))
            {
                return members;
            }
        }
    })
    .await
    .expect("membership acknowledged")
}

async fn send(socket: &mut Socket, message: &ClientMessage) {
    socket
        .send(Message::Text(
            serde_json::to_string(message).expect("json").into(),
        ))
        .await
        .expect("send");
}

#[tokio::test]
#[cfg_attr(
    not(windows),
    ignore = "the live relay probe runs in the Windows quality gate; opt in elsewhere"
)]
async fn live_relay_recovers_from_refusal_and_relays_an_encrypted_pick() {
    if let Err(error) = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_test_writer()
        .try_init()
    {
        eprintln!("Live probe logger already initialized: {error}");
    }
    let token = PartyToken::generate(1, 0).expect("random probe room");
    let url = format!(
        "{}/room?key={}",
        super::super::config::DEFAULT_RELAY_URL,
        room_id(token.key())
    );
    let refused = || Error::Io(io::Error::from(io::ErrorKind::ConnectionRefused));
    let probe = async {
        let (mut first, _) = recover(&url, WebSocketConfig::default(), refused())
            .await
            .expect("first TLS connection");
        let (mut second, _) = recover(&url, WebSocketConfig::default(), refused())
            .await
            .expect("second TLS connection");
        send(
            &mut first,
            &ClientMessage::Join {
                summoner_id: 1,
                summoner_name: String::new(),
            },
        )
        .await;
        send(
            &mut second,
            &ClientMessage::Join {
                summoner_id: 2,
                summoner_name: String::new(),
            },
        )
        .await;
        receive(&mut second, false).await;
        let cipher = RoomCipher::new(token.key());
        let pick = Announcement {
            member_id: 1,
            puuid: "dekan-connectivity-probe".into(),
            champion_id: 238,
            skin_id: 238_010,
            chroma_id: None,
        };
        send(
            &mut first,
            &ClientMessage::Skin {
                skin: Some(pick.seal(&cipher).expect("encrypt")),
            },
        )
        .await;
        let members = receive(&mut second, true).await;
        let member = members
            .iter()
            .find(|member| member.summoner_id.as_u64() == Some(1))
            .expect("first member");
        let peer = open_member(member, &cipher, 2)
            .expect("decrypt")
            .expect("peer");
        assert_eq!((peer.champion_id, peer.skin_id), (238, 238_010));
        first.close(None).await.expect("first close");
        second.close(None).await.expect("second close");
    };
    tokio::time::timeout(Duration::from_secs(45), probe)
        .await
        .expect("bounded live probe");
}
