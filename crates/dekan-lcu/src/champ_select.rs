use dekan_core::party::TeamMember;
use dekan_core::phase::GamePhase;
use dekan_core::selection::{ChampionId, SkinId};
use dekan_core::state::{StateSender, set_champion, set_phase, set_selected_skin, set_team};
use serde::{Deserialize, Serialize};

use crate::error::LcuError;
use tracing::{debug, info};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChampSelectPlayer {
    pub cell_id: i64,

    pub champion_id: ChampionId,

    pub selected_skin_id: SkinId,

    pub assigned_position: Option<String>,

    #[serde(default)]
    pub puuid: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChampSelectTimer {
    #[serde(default)]
    pub adjusted_time_left_in_phase: i64,

    pub phase: Option<String>,

    pub total_time_in_phase: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChampSelectAction {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub actor_cell_id: i64,
    #[serde(default, rename = "type")]
    pub action_type: String,
    #[serde(default)]
    pub completed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChampSelectSession {
    pub local_player_cell_id: i64,

    #[serde(default)]
    pub my_team: Vec<ChampSelectPlayer>,

    #[serde(default)]
    pub actions: Vec<Vec<ChampSelectAction>>,

    pub timer: Option<ChampSelectTimer>,

    #[serde(default)]
    pub is_custom_game: bool,
}

impl ChampSelectSession {
    #[must_use]
    pub fn local_player(&self) -> Option<&ChampSelectPlayer> {
        self.my_team
            .iter()
            .find(|p| p.cell_id == self.local_player_cell_id)
    }

    #[must_use]
    pub fn local_pick_action(&self) -> Option<&ChampSelectAction> {
        self.actions.iter().flatten().find(|action| {
            action.actor_cell_id == self.local_player_cell_id && action.action_type == "pick"
        })
    }

    pub fn local_selection(&self) -> Result<(ChampionId, SkinId), LcuError> {
        let player = self.local_player().ok_or(LcuError::LocalPlayerNotFound)?;
        Ok((player.champion_id, player.selected_skin_id))
    }

    #[must_use]
    pub fn is_finalization(&self) -> bool {
        self.timer
            .as_ref()
            .and_then(|t| t.phase.as_deref())
            .map(|p| p.eq_ignore_ascii_case("FINALIZATION"))
            .unwrap_or(false)
    }
}

pub fn apply_session_to_state(state_tx: &StateSender, session: &ChampSelectSession) {
    let local_puuid = session
        .local_player()
        .map(|p| p.puuid.clone())
        .filter(|p| !p.is_empty())
        .or_else(|| state_tx.borrow().local_puuid.clone());
    let team: Vec<TeamMember> = session
        .my_team
        .iter()
        .filter(|p| !p.puuid.is_empty() && p.champion_id > 0)
        .map(|p| TeamMember {
            puuid: p.puuid.clone(),
            champion_id: p.champion_id,
        })
        .collect();
    set_team(state_tx, local_puuid, team);

    match session.local_selection() {
        Ok((champ_id, skin_id)) => {
            let (previous_champion, previous_skin, previous_phase) = {
                let state = state_tx.borrow();
                (state.champion_id, state.selected_skin_id, state.phase)
            };

            if champ_id > 0 && previous_champion != Some(champ_id) {
                info!(
                    champion_id = champ_id,
                    previous = ?previous_champion,
                    cell_id = session.local_player_cell_id,
                    "Champ select: champion changed"
                );
                set_champion(state_tx, champ_id);
            }

            if skin_id > 0 && previous_skin != Some(skin_id) {
                info!(
                    skin_id,
                    champion_id = champ_id,
                    previous = ?previous_skin,
                    "Champ select: LCU skin selection changed"
                );
                set_selected_skin(state_tx, skin_id);
            }

            if session.is_finalization() && previous_phase != GamePhase::Finalization {
                info!(
                    champion_id = champ_id,
                    time_left_ms = session
                        .timer
                        .as_ref()
                        .map(|t| t.adjusted_time_left_in_phase),
                    "Champ select entered finalization"
                );
                set_phase(state_tx, GamePhase::Finalization);
            }
        }

        Err(e) => {
            debug!(
                error = %e,
                cell_id = session.local_player_cell_id,
                team_size = session.my_team.len(),
                "Champ select update carried no local player yet"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dekan_core::phase::QueueType;
    use dekan_core::state::{enter_champ_select, new_state_channel};

    #[test]
    fn test_parse_session_and_extract_local_selection() {
        let json = r#"{
            "localPlayerCellId": 2,
            "myTeam": [
                {
                    "cellId": 0,
                    "championId": 103,
                    "selectedSkinId": 103001,
                    "assignedPosition": "middle"
                },
                {
                    "cellId": 2,
                    "championId": 21,
                    "selectedSkinId": 21069,
                    "assignedPosition": "bottom"
                }
            ],
            "timer": {
                "adjustedTimeLeftInPhase": 12000,
                "phase": "FINALIZATION",
                "totalTimeInPhase": 30000
            },
            "isCustomGame": false
        }"#;

        let session: ChampSelectSession = serde_json::from_str(json).expect("valid session json");
        assert_eq!(session.local_player_cell_id, 2);
        assert!(session.is_finalization());

        let (champ, skin) = session.local_selection().expect("local player exists");
        assert_eq!(champ, 21);
        assert_eq!(skin, 21069);

        let (tx, rx) = new_state_channel();
        enter_champ_select(&tx, QueueType::Draft);

        apply_session_to_state(&tx, &session);

        let state = rx.borrow();
        assert_eq!(state.champion_id, Some(21));
        assert_eq!(state.selected_skin_id, Some(21069));
        assert_eq!(state.phase, GamePhase::Finalization);
    }

    #[test]
    fn test_repeated_identical_session_does_not_wake_the_app() {
        let json = r#"{
            "localPlayerCellId": 0,
            "myTeam": [ { "cellId": 0, "championId": 238, "selectedSkinId": 238000 } ],
            "isCustomGame": false
        }"#;
        let session: ChampSelectSession = serde_json::from_str(json).expect("valid session json");

        let (tx, mut rx) = new_state_channel();
        apply_session_to_state(&tx, &session);
        assert!(
            rx.has_changed().expect("sender alive"),
            "the first update is a real change"
        );
        let _ = rx.borrow_and_update(); // ignore-ok: test only consumes the change notification

        apply_session_to_state(&tx, &session);
        assert!(
            !rx.has_changed().expect("sender alive"),
            "an identical session must not publish anything"
        );

        let state = rx.borrow();
        assert_eq!(state.champion_id, Some(238));
        assert_eq!(state.selected_skin_id, Some(238_000));
    }

    #[test]
    fn test_session_missing_local_player() {
        let json = r#"{
            "localPlayerCellId": 99,
            "myTeam": [
                { "cellId": 0, "championId": 1, "selectedSkinId": 1000 }
            ],
            "isCustomGame": false
        }"#;

        let session: ChampSelectSession = serde_json::from_str(json).unwrap();
        assert!(session.local_player().is_none());
        assert!(matches!(
            session.local_selection(),
            Err(LcuError::LocalPlayerNotFound)
        ));
    }
}
