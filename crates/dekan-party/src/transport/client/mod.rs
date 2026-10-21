use std::collections::HashMap;
use std::time::Duration;

use chacha20poly1305::aead::Generate;
use dekan_core::party::PartyStatus;
use dekan_core::state::{AppState, StateReceiver, StateSender, set_party_peers, set_party_status};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::crypto::{RoomCipher, room_id};
use crate::error::PartyError;
use crate::protocol::{Announcement, ClientMessage, RelayMessage, open_member};
use crate::token::PartyToken;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

const PING_INTERVAL: Duration = Duration::from_secs(30);

const MAX_MESSAGE_BYTES: usize = 256 * 1024;

const CLOSE_REPLACED: u16 = 4000;

const CLOSE_ROOM_FULL: u16 = 4009;

const BACKOFF_MIN: Duration = Duration::from_secs(1);
const BACKOFF_MAX: Duration = Duration::from_secs(30);

const STABLE_SESSION: Duration = Duration::from_secs(60);

fn retry_backoff(backoff: Duration, session_lasted: Duration) -> Duration {
    if session_lasted >= STABLE_SESSION {
        BACKOFF_MIN
    } else {
        backoff
    }
}

enum SessionEnd {
    Cancelled,
    ClosedByRelay(String),
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
        let mut announce_connection = true;

        info!(
            member_id = self.member_id,
            relay = %self.relay_url,
            "Party: joining the room"
        );

        let exit = loop {
            set_party_status(&self.state_tx, PartyStatus::Connecting);
            let started = tokio::time::Instant::now();
            let ended = self
                .session(&url, &cipher, &cancel, &mut announce_connection)
                .await;
            backoff = retry_backoff(backoff, started.elapsed());
            let reason = match ended {
                Ok(SessionEnd::Cancelled) => break PartyExit::Left,
                Err(PartyError::RoomFull) => {
                    warn!("Party: the relay refused us because the room is full; not retrying");
                    break PartyExit::RoomFull;
                }
                Ok(SessionEnd::ClosedByRelay(detail)) => detail,
                Err(e @ PartyError::Crypto(_)) => {
                    error!(error = %e, "Party: could not seal our pick for the room");
                    e.to_string()
                }
                Err(e) => e.to_string(),
            };
            let retry_in_ms = backoff.as_millis();
            if backoff == BACKOFF_MIN {
                warn!(reason = %reason, retry_in_ms, "Party: lost the relay; reconnecting");
                announce_connection = true;
            } else {
                debug!(reason = %reason, retry_in_ms, "Party: the relay is still unreachable");
            }
            set_party_status(&self.state_tx, PartyStatus::Error { reason });
            set_party_peers(&self.state_tx, Vec::new());

            let jitter = match u32::try_generate() {
                Ok(random) => Duration::from_millis(u64::from(random % 500)),
                Err(e) => {
                    warn!(error = %e, "Party: no randomness for the reconnect jitter; retrying without it");
                    Duration::ZERO
                }
            };
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
        announce_connection: &mut bool,
    ) -> Result<SessionEnd, PartyError> {
        let config = WebSocketConfig::default()
            .max_message_size(Some(MAX_MESSAGE_BYTES))
            .max_frame_size(Some(MAX_MESSAGE_BYTES));
        let connect = super::connection::connect(url, config);
        let connected = tokio::select! {
            () = cancel.cancelled() => return Ok(SessionEnd::Cancelled),
            connected = tokio::time::timeout(CONNECT_TIMEOUT, connect) => connected,
        };
        let (stream, _) = connected
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
        if std::mem::take(announce_connection) {
            info!(member_id = self.member_id, "Party: connected to the room");
        } else {
            debug!(member_id = self.member_id, "Party: reconnected to the room");
        }

        let mut state_rx = self.state_rx.clone();
        let mut announced: Option<Option<Announcement>> = None;
        let mut rejections: HashMap<u64, String> = HashMap::new();
        let mut last_members: Option<String> = None;
        let mut sharing = state_rx.borrow().party_share;
        let mut ping = tokio::time::interval(PING_INTERVAL);
        ping.tick().await;
        let mut last_heard = tokio::time::Instant::now();
        let mut warned_unreadable = false;

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
                    if last_heard.elapsed() > PING_INTERVAL * 2 {
                        return Err(PartyError::Relay("the relay stopped answering".into()));
                    }
                    write
                        .send(Message::Text("ping".into()))
                        .await
                        .map_err(|e| PartyError::Relay(e.to_string()))?;
                }
                changed = state_rx.changed() => {
                    if changed.is_err() {
                        return Ok(SessionEnd::Cancelled);
                    }
                    let now_sharing = state_rx.borrow().party_share;
                    if now_sharing != sharing {
                        sharing = now_sharing;
                        if let Some(text) = &last_members {
                            if let Err(e) = self.handle_relay_text(text, cipher, &mut rejections) {
                                debug!(error = %e, "Party: kept relay message not understood; ignored");
                            }
                        }
                    }
                }
                incoming = read.next() => {
                    match incoming {
                        Some(Ok(Message::Text(text))) => {
                            last_heard = tokio::time::Instant::now();
                            if text == "pong" {
                                continue;
                            }
                            if let Err(e) = self.handle_relay_text(&text, cipher, &mut rejections)
                                && !std::mem::replace(&mut warned_unreadable, true)
                            {
                                warn!(error = %e, "Party: the relay sent a message this version does not understand; ignoring such messages");
                            }
                            last_members = Some(text.to_string());
                        }
                        Some(Ok(Message::Close(frame))) => {
                            let Some(frame) = frame else {
                                return Ok(SessionEnd::ClosedByRelay("closed by the relay".into()));
                            };
                            let code = u16::from(frame.code);
                            if frame.code == CloseCode::Library(CLOSE_ROOM_FULL) {
                                return Err(PartyError::RoomFull);
                            }
                            let note = if frame.code == CloseCode::Library(CLOSE_REPLACED) {
                                "another connection of this member replaced this one"
                            } else {
                                "closed by the relay"
                            };
                            return Ok(SessionEnd::ClosedByRelay(format!(
                                "{note} (code {code}, reason {:?})",
                                frame.reason.as_str()
                            )));
                        }
                        None => return Ok(SessionEnd::ClosedByRelay("connection closed without a close frame".into())),
                        Some(Ok(Message::Pong(_))) => last_heard = tokio::time::Instant::now(),
                        Some(Ok(other)) => {
                            debug!(kind = ?other, "Party: unexpected frame from the relay; ignored");
                        }
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
    ) -> Result<(), serde_json::Error> {
        let RelayMessage::Members { members } = serde_json::from_str(text)?;

        let mut peers = Vec::new();
        if self.state_rx.borrow().party_share {
            let present: Vec<u64> = members
                .iter()
                .filter_map(|m| m.summoner_id.as_u64())
                .collect();
            rejections
                .retain(|id, _| present.contains(id) || *id > u64::MAX - members.len() as u64);
            for (index, member) in members.iter().enumerate() {
                let member_id = member
                    .summoner_id
                    .as_u64()
                    .unwrap_or(u64::MAX - index as u64);
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
        Ok(())
    }
}

fn announcement(state: &AppState, member_id: u64) -> Option<Announcement> {
    if !state.party_share || !(state.phase.is_champ_select() || state.phase.is_in_game()) {
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
mod tests;
