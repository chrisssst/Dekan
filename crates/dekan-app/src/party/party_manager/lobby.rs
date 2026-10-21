use std::collections::HashSet;

use dekan_core::party::{PartyStatus, lobby_room, may_ask_to_share};
use dekan_core::state::set_party_share;
use dekan_party::token::PartyToken;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use super::{PartyManager, Room, Running};

#[derive(Default)]
pub(super) struct LobbyFollow {
    accepted: HashSet<String>,
    declined: HashSet<String>,
    invite: Option<(String, JoinHandle<bool>)>,
}

impl LobbyFollow {
    pub(super) fn decline(&mut self, party_id: String) {
        self.declined.insert(party_id);
    }

    pub(super) fn dismiss(&mut self) {
        if self.invite.take().is_some() {
            drop(tokio::task::spawn_blocking(|| {
                if let Err(e) = dekan_platform::party_dialog::dismiss_lobby_invite() {
                    debug!(error = %e, "Party: the interface is gone; the lobby invite went with it");
                }
            }));
        }
    }

    pub(super) async fn next_answer(&mut self) -> (String, bool) {
        let Some((_, answer)) = self.invite.as_mut() else {
            return std::future::pending().await;
        };
        let accepted = answer.await.unwrap_or_else(|e| {
            warn!(error = %e, "Party: the lobby invite task failed; treated as declined");
            false
        });
        let party_id = self.invite.take().map(|(id, _)| id).unwrap_or_default();
        (party_id, accepted)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LobbyStep {
    Stay,
    Switch,
    Ask { friends: usize },
}

pub(super) fn lobby_step(
    current: Option<&str>,
    wanted: Option<&str>,
    members: usize,
    sharing: bool,
    may_ask: bool,
    asking: bool,
) -> LobbyStep {
    if current != wanted {
        return LobbyStep::Switch;
    }
    if current.is_none() || sharing || members < 2 || !may_ask || asking {
        return LobbyStep::Stay;
    }
    LobbyStep::Ask {
        friends: members - 1,
    }
}

pub(super) fn invite_outdated(invite: Option<&str>, wanted: Option<&str>, may_ask: bool) -> bool {
    invite.is_some_and(|party_id| !may_ask || wanted != Some(party_id))
}

fn lobby_of(running: Option<&Running>) -> Option<&str> {
    match running.map(|r| &r.room) {
        Some(Room::Lobby(party_id)) => Some(party_id),
        _ => None,
    }
}

impl PartyManager {
    pub(super) async fn follow_lobby(
        &self,
        running: &mut Option<Running>,
        lobby: &mut LobbyFollow,
    ) {
        if super::in_code_room(running.as_ref()) {
            return;
        }
        let current = lobby_of(running.as_ref()).map(str::to_owned);
        let (wanted, members, sharing, may_ask) = {
            let state = self.state_rx.borrow();
            let members = match state.party_status {
                PartyStatus::Connected { members } => members,
                _ => 0,
            };
            let wanted = lobby_room(current.as_deref(), state.lobby_party.as_ref(), state.phase)
                .filter(|party_id| !lobby.declined.contains(party_id));
            (
                wanted,
                members,
                state.party_share,
                may_ask_to_share(state.phase),
            )
        };
        let invite = lobby.invite.as_ref().map(|(party_id, _)| party_id.as_str());
        if invite_outdated(invite, wanted.as_deref(), may_ask) {
            info!(
                "Party: the lobby invite closed unanswered; the queue started or the lobby changed"
            );
            lobby.dismiss();
        }
        let asking = lobby.invite.is_some();
        match lobby_step(
            current.as_deref(),
            wanted.as_deref(),
            members,
            sharing,
            may_ask,
            asking,
        ) {
            LobbyStep::Stay => {}
            LobbyStep::Switch => {
                if current.is_some() {
                    info!(
                        another_lobby = wanted.is_some(),
                        "Party: the lobby changed; its room closes"
                    );
                }
                self.stop(running).await;
                if let Some(party_id) = wanted {
                    self.open_lobby_room(running, party_id, lobby);
                }
            }
            LobbyStep::Ask { friends } => {
                if let Some(party_id) = current {
                    info!(
                        friends,
                        "Party: friends in the lobby run Dekan; asking to share picks"
                    );
                    let answer = tokio::task::spawn_blocking(move || {
                        dekan_platform::party_dialog::show_lobby_invite().unwrap_or_else(|e| {
                            warn!(error = %e, "Party: the lobby invite could not be shown");
                            false
                        })
                    });
                    lobby.invite = Some((party_id, answer));
                }
            }
        }
    }

    fn open_lobby_room(
        &self,
        running: &mut Option<Running>,
        party_id: String,
        lobby: &mut LobbyFollow,
    ) {
        let relay = match dekan_party::config::relay_url(&self.state_dir) {
            Ok(relay) => relay,
            Err(e) => {
                warn!(error = %e, "Party: no relay configured; the lobby room is not opened");
                lobby.decline(party_id);
                return;
            }
        };
        let token = PartyToken::for_lobby(&party_id);
        let share = lobby.accepted.contains(&party_id);
        if self.start(running, relay, token, Room::Lobby(party_id.clone())) {
            info!(share, "Party: joined the room of the lobby");
            if share {
                set_party_share(&self.state_tx, true);
            }
        } else {
            lobby.decline(party_id);
        }
    }

    pub(super) async fn answer_invite(
        &self,
        (party_id, accepted): (String, bool),
        running: &mut Option<Running>,
        lobby: &mut LobbyFollow,
    ) {
        let still_here = lobby_of(running.as_ref()) == Some(party_id.as_str());
        if accepted {
            info!("Party: sharing picks with the friends in the lobby");
            if still_here {
                set_party_share(&self.state_tx, true);
            }
            lobby.accepted.insert(party_id);
        } else {
            info!("Party: the lobby invite was declined; the lobby room closes");
            if still_here {
                self.stop(running).await;
            }
            lobby.decline(party_id);
        }
        self.follow_lobby(running, lobby).await;
    }
}
