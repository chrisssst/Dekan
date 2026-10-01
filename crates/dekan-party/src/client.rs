use std::collections::HashMap;
use std::time::Duration;

use dekan_core::party::PartyStatus;
use dekan_core::state::{AppState, StateReceiver, StateSender, set_party_peers, set_party_status};
use chacha20poly1305::aead::Generate;
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::crypto::{RoomCipher, room_id};
use crate::error::PartyError;
use crate::protocol::{Announcement, ClientMessage, RelayMessage, open_member};
use crate::token::PartyToken;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

const PING_INTERVAL: Duration = Duration::from_secs(30);

const MAX_MESSAGE_BYTES: usize = 256 * 1024;

const BACKOFF_MIN: Duration = Duration::from_secs(1);
const BACKOFF_MAX: Duration = Duration::from_secs(30);

enum SessionEnd {
    Cancelled,
    ClosedByRelay,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartyExit {
    Left,

    RoomFull,
}

pub struct PartyClient {
    relay_url: String,
    token: PartyToken,
    member_id: u64,
    state_tx: StateSender,
    state_rx: StateReceiver,
}

impl PartyClient {
    #[must_use]
    pub fn new(
        relay_url: String,
        token: PartyToken,
        member_id: u64,
        state_tx: StateSender,
        state_rx: StateReceiver,
    ) -> Self {
        Self {
            relay_url,
            token,
            member_id,
            state_tx,
            state_rx,
        }
    }

    pub async fn run(self, cancel: CancellationToken) -> PartyExit {
        let cipher = RoomCipher::new(self.token.key());
        let url = format!("{}/room?key={}", self.relay_url, room_id(self.token.key()));
        let mut backoff = BACKOFF_MIN;

        info!(
            member_id = self.member_id,
            relay = %self.relay_url,
            "Party: joining the room"
        );

        let exit = loop {
            set_party_status(&self.state_tx, PartyStatus::Connecting);
            match self.session(&url, &cipher, &cancel).await {
                Ok(SessionEnd::Cancelled) => break PartyExit::Left,
                Err(PartyError::RoomFull) => {
                    warn!("Party: the relay refused us because the room is full; not retrying");
                    break PartyExit::RoomFull;
                }
                Ok(SessionEnd::ClosedByRelay) => {
                    warn!("Party: the relay closed the connection; reconnecting");
                    set_party_status(
                        &self.state_tx,
                        PartyStatus::Error {
                            reason: "conexão encerrada pelo relay".into(),
                        },
                    );
                }
                Err(e) => {
                    warn!(error = %e, retry_in_ms = backoff.as_millis(), "Party: connection failed");
                    set_party_status(
                        &self.state_tx,
                        PartyStatus::Error {
                            reason: e.to_string(),
                        },
                    );
                }
            }
            set_party_peers(&self.state_tx, Vec::new());

            // Jitter only spreads reconnects; without randomness a plain backoff is fine.
            let jitter = Duration::from_millis(u64::from(u32::try_generate().unwrap_or(0) % 500));
            tokio::select! {
                () = cancel.cancelled() => break PartyExit::Left,
                () = tokio::time::sleep(backoff + jitter) => {}
            }
            backoff = (backoff * 2).min(BACKOFF_MAX);
        };

        set_party_peers(&self.state_tx, Vec::new());
        set_party_status(&self.state_tx, PartyStatus::Off);
        info!(?exit, "Party: left the room");
        exit
    }

    async fn session(
        &self,
        url: &str,
        cipher: &RoomCipher,
        cancel: &CancellationToken,
    ) -> Result<SessionEnd, PartyError> {
        let config = WebSocketConfig::default()
            .max_message_size(Some(MAX_MESSAGE_BYTES))
            .max_frame_size(Some(MAX_MESSAGE_BYTES));
        let connect = tokio_tungstenite::connect_async_with_config(url, Some(config), false);
        let (stream, _) = tokio::time::timeout(CONNECT_TIMEOUT, connect)
            .await
            .map_err(|_| PartyError::Relay("connection timed out".into()))?
            .map_err(|e| match e {
                tokio_tungstenite::tungstenite::Error::Http(response)
                    if response.status().as_u16() == 409 =>
                {
                    PartyError::RoomFull
                }
                other => PartyError::Relay(other.to_string()),
            })?;
        let (mut write, mut read) = stream.split();

        let send = |message: &ClientMessage| -> Result<Message, PartyError> {
            Ok(Message::Text(serde_json::to_string(message)?.into()))
        };

        write
            .send(send(&ClientMessage::Join {
                summoner_id: self.member_id,
                summoner_name: String::new(),
            })?)
            .await
            .map_err(|e| PartyError::Relay(e.to_string()))?;
        info!(member_id = self.member_id, "Party: connected to the room");

        let mut state_rx = self.state_rx.clone();
        let mut announced: Option<Option<Announcement>> = None;
        let mut rejections: HashMap<u64, String> = HashMap::new();
        let mut ping = tokio::time::interval(PING_INTERVAL);
        ping.tick().await;

        loop {
            let current = announcement(&state_rx.borrow_and_update(), self.member_id);
            if announced.as_ref() != Some(&current) {
                let skin = match &current {
                    Some(a) => Some(a.seal(cipher)?),
                    None => None,
                };
                write
                    .send(send(&ClientMessage::Skin { skin })?)
                    .await
                    .map_err(|e| PartyError::Relay(e.to_string()))?;
                info!(
                    champion_id = ?current.as_ref().map(|a| a.champion_id),
                    skin_id = ?current.as_ref().map(|a| a.skin_id),
                    chroma_id = ?current.as_ref().and_then(|a| a.chroma_id),
                    "Party: our pick announced to the room"
                );
                announced = Some(current);
            }

            tokio::select! {
                () = cancel.cancelled() => {

                    if let Ok(leave) = send(&ClientMessage::Leave) {

                        // ignore-ok: leaving on shutdown; a failed send means the socket is already gone
                        let _ = write.send(leave).await;
                    }

                    // ignore-ok: same as above, the close handshake is courtesy
                    let _ = write.close().await;
                    return Ok(SessionEnd::Cancelled);
                }
                _ = ping.tick() => {
                    write
                        .send(Message::Text("ping".into()))
                        .await
                        .map_err(|e| PartyError::Relay(e.to_string()))?;
                }
                changed = state_rx.changed() => {
                    if changed.is_err() {
                        return Ok(SessionEnd::Cancelled);
                    }
                }
                incoming = read.next() => {
                    match incoming {
                        Some(Ok(Message::Text(text))) => {
                            if text == "pong" {
                                continue;
                            }
                            self.handle_relay_text(&text, cipher, &mut rejections);
                        }
                        Some(Ok(Message::Close(_))) | None => return Ok(SessionEnd::ClosedByRelay),
                        Some(Ok(_)) => {}
                        Some(Err(e)) => return Err(PartyError::Relay(e.to_string())),
                    }
                }
            }
        }
    }

    fn handle_relay_text(
        &self,
        text: &str,
        cipher: &RoomCipher,
        rejections: &mut HashMap<u64, String>,
    ) {
        let RelayMessage::Members { members } = match serde_json::from_str(text) {
            Ok(message) => message,
            Err(e) => {
                debug!(error = %e, "Party: relay message not understood; ignored");
                return;
            }
        };

        let mut peers = Vec::new();
        for member in &members {
            let member_id = member.summoner_id.as_u64().unwrap_or(0);
            match open_member(member, cipher, self.member_id) {
                Ok(Some(peer)) => {
                    rejections.remove(&member_id);
                    peers.push(peer);
                }
                Ok(None) => {
                    rejections.remove(&member_id);
                }
                Err(e) => {
                    let reason = e.to_string();
                    if rejections.get(&member_id) != Some(&reason) {
                        warn!(member_id, reason = %reason, "Party: a member's announcement was refused");
                        rejections.insert(member_id, reason);
                    }
                }
            }
        }
        peers.sort_by_key(|p| p.member_id);

        if self.state_rx.borrow().party_peers != peers {
            info!(
                members = members.len(),
                announcing = peers.len(),
                picks = ?peers.iter().map(|p| (p.champion_id, p.entry_id())).collect::<Vec<_>>(),
                "Party: room picks changed"
            );
        }
        set_party_status(
            &self.state_tx,
            PartyStatus::Connected {
                members: members.len(),
            },
        );
        set_party_peers(&self.state_tx, peers);
    }
}

fn announcement(state: &AppState, member_id: u64) -> Option<Announcement> {
    if !(state.phase.is_champ_select() || state.phase.is_in_game()) {
        return None;
    }
    let champion_id = state.champion_id?;
    let target = state
        .overlay_target
        .as_ref()
        .filter(|t| t.matches_champion(champion_id))?;
    let puuid = state.local_puuid.clone()?;
    let announcement = Announcement {
        member_id,
        puuid,
        champion_id,
        skin_id: target.skin_id,
        chroma_id: target.chroma_id,
    };
    if target.package_entry_id() == champion_id * 1000 || announcement.validate().is_err() {
        return None;
    }
    Some(announcement)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dekan_core::overlay::OverlayTarget;
    use dekan_core::phase::GamePhase;

    fn picking_103() -> AppState {
        AppState {
            phase: GamePhase::ChampSelect,
            champion_id: Some(103),
            local_puuid: Some("me-1".into()),
            overlay_target: Some(OverlayTarget {
                champion_id: 103,
                skin_id: 103_015,
                chroma_id: None,
            }),
            ..Default::default()
        }
    }

    #[test]
    fn test_a_pick_is_announced_only_while_its_match_is_live() {
        let mut state = picking_103();
        for phase in [
            GamePhase::ChampSelect,
            GamePhase::Finalization,
            GamePhase::GameStart,
            GamePhase::InProgress,
            GamePhase::Reconnect,
        ] {
            state.phase = phase;
            assert!(
                announcement(&state, 7).is_some(),
                "{phase:?} belongs to the match the pick was made in"
            );
        }

        for phase in [
            GamePhase::None,
            GamePhase::Lobby,
            GamePhase::Matchmaking,
            GamePhase::ReadyCheck,
            GamePhase::CheckedIntoTournament,
            GamePhase::WaitingForStats,
            GamePhase::PreEndOfGame,
            GamePhase::EndOfGame,
            GamePhase::FailedToLaunch,
            GamePhase::TerminatedInError,
        ] {
            state.phase = phase;
            assert_eq!(
                announcement(&state, 7),
                None,
                "{phase:?}: a champion here can only be a stale one"
            );
        }
    }

    #[test]
    fn test_we_announce_only_a_valid_pick_for_our_own_champion() {
        let mut state = AppState {
            phase: GamePhase::ChampSelect,
            champion_id: Some(103),
            local_puuid: Some("me-1".into()),
            overlay_target: Some(OverlayTarget {
                champion_id: 103,
                skin_id: 103_015,
                chroma_id: None,
            }),
            ..Default::default()
        };
        let a = announcement(&state, 7).expect("announces");
        assert_eq!((a.member_id, a.champion_id, a.skin_id), (7, 103, 103_015));

        state.champion_id = Some(1);
        assert_eq!(
            announcement(&state, 7),
            None,
            "a stale pick for another champion is not sent"
        );

        state.champion_id = Some(60_103);
        state.overlay_target = Some(OverlayTarget {
            champion_id: 60_103,
            skin_id: 103_015,
            chroma_id: None,
        });
        assert_eq!(
            announcement(&state, 7),
            None,
            "Rift Classic is not part of party"
        );

        state.champion_id = Some(103);
        state.overlay_target = Some(OverlayTarget {
            champion_id: 103,
            skin_id: 103_015,
            chroma_id: None,
        });
        state.local_puuid = None;
        assert_eq!(
            announcement(&state, 7),
            None,
            "without our PUUID nobody could verify us"
        );
    }
}
