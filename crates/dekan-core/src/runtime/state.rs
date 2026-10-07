use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use crate::lobby::{LobbyPicks, LobbySlot};
use crate::mods::ModSelection;
use crate::overlay::OverlayTarget;
use crate::party::{PartyPeer, PartyStatus, TeamMember};
use crate::phase::GamePhase;
use crate::selection::{ChampionId, SkinId};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum InjectionStatus {
    #[default]
    Idle,

    Pending,

    Confirmed,

    Unconfirmed,

    Failed {
        error: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppState {
    pub phase: GamePhase,

    pub champion_id: Option<ChampionId>,

    pub selected_skin_id: Option<SkinId>,

    pub overlay_target: Option<OverlayTarget>,

    pub lcu_connected: bool,

    pub injection: InjectionStatus,

    pub mods: ModSelection,

    pub local_puuid: Option<String>,

    pub team: Vec<TeamMember>,

    pub party_status: PartyStatus,

    pub party_hosting: bool,

    pub party_peers: Vec<PartyPeer>,

    pub lobby: Option<LobbyPicks>,

    pub queue: Option<QueueInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueInfo {
    pub queue_id: u32,
    pub game_mode: String,
    pub map_id: u32,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            phase: GamePhase::None,
            champion_id: None,
            selected_skin_id: None,
            overlay_target: None,
            lcu_connected: false,
            injection: InjectionStatus::Idle,
            mods: ModSelection::default(),
            local_puuid: None,
            team: Vec::new(),
            party_status: PartyStatus::Off,
            party_hosting: false,
            party_peers: Vec::new(),
            lobby: None,
            queue: None,
        }
    }
}

pub type StateReceiver = watch::Receiver<AppState>;

pub type StateSender = watch::Sender<AppState>;

#[must_use]
pub fn new_state_channel() -> (StateSender, StateReceiver) {
    watch::channel(AppState::default())
}

fn reset_match(state: &mut AppState) {
    state.champion_id = None;
    state.selected_skin_id = None;
    state.overlay_target = None;
    state.injection = InjectionStatus::Idle;
    state.local_puuid = None;
    state.team.clear();
    focus_lobby(state, None);
}

fn focus_lobby(state: &mut AppState, wanted: Option<ChampionId>) {
    let Some(lobby) = state.lobby.as_ref() else {
        return;
    };
    let focus = wanted
        .filter(|champion| lobby.holds(*champion))
        .or_else(|| lobby.champions().first().copied());
    state.champion_id = focus;
    state.selected_skin_id = focus.and_then(|champion| lobby.skin_of(champion));
    state.overlay_target = focus.and_then(|champion| lobby.target_for(champion).cloned());
}

fn starts_new_match(from: GamePhase, to: GamePhase) -> bool {
    if to.is_champ_select() {
        return !from.is_champ_select();
    }
    from != to
        && matches!(
            to,
            GamePhase::Lobby | GamePhase::Matchmaking | GamePhase::ReadyCheck
        )
}

pub fn set_champion(tx: &StateSender, champion_id: ChampionId) {
    tx.send_modify(|state| {
        state.champion_id = Some(champion_id);
    });
}

pub fn set_selected_skin(tx: &StateSender, skin_id: SkinId) {
    tx.send_modify(|state| {
        state.selected_skin_id = Some(skin_id);
    });
}

pub fn set_overlay_target(tx: &StateSender, target: OverlayTarget) {
    tx.send_modify(|state| {
        if let Some(lobby) = state.lobby.as_mut() {
            lobby.remember(&target);
        }
        state.overlay_target = Some(target);
    });
}

pub fn clear_overlay_target(tx: &StateSender) {
    tx.send_modify(|state| {
        if let (Some(lobby), Some(champion)) = (state.lobby.as_mut(), state.champion_id) {
            lobby.forget(champion);
        }
        state.overlay_target = None;
    });
}

pub fn set_lobby_picks(tx: &StateSender, picks: Option<Vec<LobbySlot>>) -> bool {
    tx.send_if_modified(|state| {
        let before = (
            state.lobby.clone(),
            state.champion_id,
            state.selected_skin_id,
            state.overlay_target.clone(),
        );
        let champion_follows_lobby = state.phase.is_before_champ_select();
        match picks {
            None if !state.phase.is_between_matches() => {}
            None => {
                if state.lobby.take().is_some() && champion_follows_lobby {
                    state.champion_id = None;
                    state.selected_skin_id = None;
                    state.overlay_target = None;
                }
            }
            Some(slots) => {
                let mut lobby = state.lobby.take().unwrap_or_default();
                lobby.replace_slots(slots);
                state.lobby = Some(lobby);
                if champion_follows_lobby {
                    let current = state.champion_id;
                    focus_lobby(state, current);
                }
            }
        }
        before
            != (
                state.lobby.clone(),
                state.champion_id,
                state.selected_skin_id,
                state.overlay_target.clone(),
            )
    })
}

pub fn set_lobby_target(tx: &StateSender, target: &OverlayTarget) -> bool {
    tx.send_if_modified(|state| {
        let Some(lobby) = state.lobby.as_mut() else {
            return false;
        };
        if !lobby.holds(target.champion_id) || lobby.target_for(target.champion_id) == Some(target)
        {
            return false;
        }
        lobby.remember(target);
        if state.champion_id == Some(target.champion_id) {
            state.overlay_target = Some(target.clone());
        }
        true
    })
}

pub fn clear_lobby_target(tx: &StateSender, champion_id: ChampionId) -> bool {
    tx.send_if_modified(|state| {
        let Some(lobby) = state.lobby.as_mut() else {
            return false;
        };
        if lobby.target_for(champion_id).is_none() {
            return false;
        }
        lobby.forget(champion_id);
        if state.champion_id == Some(champion_id) {
            state.overlay_target = None;
        }
        true
    })
}

pub fn focus_lobby_champion(tx: &StateSender, champion_id: ChampionId) -> bool {
    tx.send_if_modified(|state| {
        let holds = state
            .lobby
            .as_ref()
            .is_some_and(|lobby| lobby.holds(champion_id));
        if !holds || state.champion_id == Some(champion_id) {
            return false;
        }
        focus_lobby(state, Some(champion_id));
        true
    })
}

pub fn set_queue(tx: &StateSender, queue: Option<QueueInfo>) -> bool {
    tx.send_if_modified(|state| {
        if state.queue == queue {
            return false;
        }
        state.queue = queue;
        true
    })
}

pub fn set_mod_selection(tx: &StateSender, selection: ModSelection) {
    tx.send_if_modified(|state| {
        if state.mods == selection {
            return false;
        }
        state.mods = selection;
        true
    });
}

pub fn set_team(tx: &StateSender, local_puuid: Option<String>, team: Vec<TeamMember>) {
    tx.send_if_modified(|state| {
        if state.local_puuid == local_puuid && state.team == team {
            return false;
        }
        state.local_puuid = local_puuid;
        state.team = team;
        true
    });
}

pub fn set_party_status(tx: &StateSender, status: PartyStatus) {
    tx.send_if_modified(|state| {
        if state.party_status == status {
            return false;
        }
        state.party_status = status;
        true
    });
}

pub fn set_party_hosting(tx: &StateSender, hosting: bool) {
    tx.send_if_modified(|state| {
        if state.party_hosting == hosting {
            return false;
        }
        state.party_hosting = hosting;
        true
    });
}

pub fn set_party_peers(tx: &StateSender, peers: Vec<PartyPeer>) {
    tx.send_if_modified(|state| {
        if state.party_peers == peers {
            return false;
        }
        state.party_peers = peers;
        true
    });
}

pub fn set_injection_status(tx: &StateSender, status: InjectionStatus) {
    tx.send_modify(|state| {
        state.injection = status;
    });
}

pub fn clear_for_new_game(tx: &StateSender) {
    tx.send_modify(|state| {
        state.lobby = None;
        reset_match(state);
        state.phase = GamePhase::None;
    });
}

pub fn set_phase(tx: &StateSender, phase: GamePhase) {
    tx.send_modify(|state| {
        let keeps_lobby_picks = state.lobby.is_some()
            && state.phase.is_before_champ_select()
            && (phase.is_before_champ_select() || phase.is_champ_select());
        if starts_new_match(state.phase, phase) && !keeps_lobby_picks {
            reset_match(state);
        }
        state.phase = phase;
    });
}

pub fn set_lcu_connected(tx: &StateSender, connected: bool) {
    tx.send_modify(|state| {
        state.lcu_connected = connected;
    });
}

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;
