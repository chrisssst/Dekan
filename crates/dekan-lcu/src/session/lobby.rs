use dekan_core::lobby::{LOBBY_PICK_MODES, LOBBY_PICK_QUEUES, LobbySlot};
use dekan_core::selection::{ChampionId, SkinId};
use dekan_core::state::{QueueInfo, StateSender, set_lobby_picks, set_queue};
use serde::Deserialize;
use tracing::{debug, info, warn};

use crate::client::LcuClient;
use crate::error::LcuError;

const PLAYER_SLOTS_PATH: &str = "/lol-lobby/v1/lobby/members/localMember/player-slots";

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LobbyGameConfig {
    #[serde(default)]
    pub queue_id: i64,
    #[serde(default)]
    pub game_mode: String,
    #[serde(default)]
    pub map_id: u32,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerSlot {
    #[serde(default)]
    pub champion_id: ChampionId,
    #[serde(default)]
    pub skin_id: SkinId,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LobbyMember {
    #[serde(default)]
    pub player_slots: Vec<PlayerSlot>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LobbySession {
    #[serde(default)]
    pub game_config: LobbyGameConfig,
    #[serde(default)]
    pub local_member: Option<LobbyMember>,
}

impl LobbySession {
    #[must_use]
    pub fn queue(&self) -> Option<QueueInfo> {
        let queue_id = u32::try_from(self.game_config.queue_id).ok()?;
        Some(QueueInfo {
            queue_id,
            game_mode: self.game_config.game_mode.clone(),
            map_id: self.game_config.map_id,
        })
    }

    #[must_use]
    pub fn picks_champions_in_lobby(&self) -> bool {
        let known_queue = self
            .queue()
            .is_some_and(|queue| LOBBY_PICK_QUEUES.contains(&queue.queue_id));
        let known_mode = LOBBY_PICK_MODES
            .iter()
            .any(|mode| self.game_config.game_mode.eq_ignore_ascii_case(mode));
        known_queue || known_mode
    }

    #[must_use]
    pub fn chosen_slots(&self) -> Vec<LobbySlot> {
        let mut slots: Vec<LobbySlot> = Vec::new();
        for slot in self
            .local_member
            .iter()
            .flat_map(|member| member.player_slots.iter())
            .filter(|slot| slot.champion_id > 0)
        {
            if slots.iter().all(|s| s.champion_id != slot.champion_id) {
                slots.push(LobbySlot {
                    champion_id: slot.champion_id,
                    skin_id: slot.skin_id,
                });
            }
        }
        slots
    }
}

pub fn apply_lobby_to_state(state_tx: &StateSender, lobby: Option<&LobbySession>) {
    let queue = lobby.and_then(LobbySession::queue);
    if set_queue(state_tx, queue.clone()) {
        if let Some(queue) = &queue {
            info!(
                queue_id = queue.queue_id,
                game_mode = %queue.game_mode,
                map_id = queue.map_id,
                champions_in_lobby = lobby.is_some_and(LobbySession::picks_champions_in_lobby),
                "Lobby queue"
            );
        }
    }

    let picks = lobby
        .filter(|lobby| lobby.picks_champions_in_lobby())
        .map(LobbySession::chosen_slots);
    let before = state_tx
        .borrow()
        .lobby
        .as_ref()
        .map(dekan_core::lobby::LobbyPicks::champions);
    if set_lobby_picks(state_tx, picks) {
        let after = state_tx
            .borrow()
            .lobby
            .as_ref()
            .map(dekan_core::lobby::LobbyPicks::champions);
        if after != before {
            match after {
                Some(champions) => info!(
                    champions = ?champions,
                    "Champions are picked in the lobby; the selection window follows them"
                ),
                None => debug!("The lobby no longer picks champions"),
            }
        }
    }
}

impl LcuClient {
    async fn lobby_as<T: serde::de::DeserializeOwned>(&self) -> Result<Option<T>, LcuError> {
        let url = format!("{}/lol-lobby/v2/lobby", self.base_url());
        let resp = self.http().get(&url).send().await?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !resp.status().is_success() {
            return Err(LcuError::Parse(format!(
                "/lol-lobby/v2/lobby unavailable (HTTP {})",
                resp.status().as_u16()
            )));
        }
        Ok(Some(resp.json::<T>().await?))
    }

    pub async fn get_lobby(&self) -> Result<Option<LobbySession>, LcuError> {
        self.lobby_as().await
    }

    pub async fn set_lobby_slot_skins(
        &self,
        skins: &[(ChampionId, SkinId)],
    ) -> Result<(), LcuError> {
        let lobby: serde_json::Value = self
            .lobby_as()
            .await?
            .ok_or_else(|| LcuError::Parse("there is no lobby to register skins in".into()))?;
        let mut slots = lobby
            .get("localMember")
            .and_then(|member| member.get("playerSlots"))
            .and_then(serde_json::Value::as_array)
            .cloned()
            .ok_or_else(|| LcuError::Parse("the lobby has no player slots".into()))?;

        let mut changed = false;
        for slot in &mut slots {
            let champion = slot
                .get("championId")
                .and_then(serde_json::Value::as_u64)
                .and_then(|id| u32::try_from(id).ok());
            let Some((_, skin)) = skins.iter().find(|(c, _)| Some(*c) == champion) else {
                continue;
            };
            if slot.get("skinId").and_then(serde_json::Value::as_u64) != Some(u64::from(*skin)) {
                slot["skinId"] = serde_json::Value::from(*skin);
                changed = true;
            }
        }
        if !changed {
            debug!(skins = ?skins, "The lobby slots already hold the skins to register");
            return Ok(());
        }

        let url = format!("{}{PLAYER_SLOTS_PATH}", self.base_url());
        let resp = self
            .http()
            .put(&url)
            .header("x-riot-source", "rcp-fe-lol-parties")
            .json(&slots)
            .send()
            .await?;
        let status = resp.status();
        if status.is_success() {
            info!(skins = ?skins, "Skins registered in the lobby slots");
        } else {
            warn!(
                skins = ?skins,
                status = status.as_u16(),
                "The client refused the lobby slot skins; the loading screen may show another skin"
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dekan_core::phase::GamePhase;
    use dekan_core::state::{new_state_channel, set_phase};

    const SWIFTPLAY: &str = r#"{
        "gameConfig": { "queueId": 480, "gameMode": "SWIFTPLAY", "mapId": 11 },
        "localMember": {
            "playerSlots": [
                { "championId": 238, "skinId": 238000, "positionPreference": "MIDDLE", "perks": "{}" },
                { "championId": 103, "skinId": 103007, "positionPreference": "BOTTOM" }
            ]
        }
    }"#;

    #[test]
    fn a_swiftplay_lobby_publishes_its_champions_and_queue() {
        let lobby: LobbySession = serde_json::from_str(SWIFTPLAY).expect("valid lobby");
        assert!(lobby.picks_champions_in_lobby());

        let (tx, rx) = new_state_channel();
        set_phase(&tx, GamePhase::Lobby);
        apply_lobby_to_state(&tx, Some(&lobby));
        let state = rx.borrow();
        assert_eq!(state.champion_id, Some(238));
        assert_eq!(
            state.lobby.as_ref().map(|l| l.champions()),
            Some(vec![238, 103])
        );
        assert_eq!(state.queue.as_ref().map(|q| q.map_id), Some(11));
    }

    #[test]
    fn slots_left_over_in_a_draft_lobby_do_not_turn_it_into_a_lobby_pick_mode() {
        let raw = r#"{ "gameConfig": { "queueId": 450, "gameMode": "ARAM", "mapId": 12 },
                       "localMember": { "playerSlots": [ { "championId": 238, "skinId": 238012 } ] } }"#;
        let lobby: LobbySession = serde_json::from_str(raw).expect("valid lobby");
        assert!(!lobby.picks_champions_in_lobby());

        let brawl = r#"{ "gameConfig": { "queueId": 2300, "gameMode": "BRAWL", "mapId": 35 } }"#;
        let brawl: LobbySession = serde_json::from_str(brawl).expect("valid lobby");
        assert!(brawl.picks_champions_in_lobby(), "known by its game mode");
    }

    #[test]
    fn a_draft_lobby_has_no_slots_and_picks_nothing() {
        let raw = r#"{ "gameConfig": { "queueId": 400, "gameMode": "CLASSIC", "mapId": 11 },
                       "localMember": { "playerSlots": [] } }"#;
        let lobby: LobbySession = serde_json::from_str(raw).expect("valid lobby");
        assert!(!lobby.picks_champions_in_lobby());

        let (tx, rx) = new_state_channel();
        set_phase(&tx, GamePhase::Lobby);
        apply_lobby_to_state(&tx, Some(&lobby));
        assert!(rx.borrow().lobby.is_none());
        assert_eq!(rx.borrow().queue.as_ref().map(|q| q.queue_id), Some(400));
    }

    #[test]
    fn a_lobby_pick_queue_opens_before_any_champion_is_chosen() {
        let raw = r#"{ "gameConfig": { "queueId": 490, "gameMode": "CLASSIC", "mapId": 11 } }"#;
        let lobby: LobbySession = serde_json::from_str(raw).expect("valid lobby");
        assert!(lobby.picks_champions_in_lobby());
        assert!(lobby.chosen_slots().is_empty());

        let (tx, rx) = new_state_channel();
        set_phase(&tx, GamePhase::Lobby);
        apply_lobby_to_state(&tx, Some(&lobby));
        assert!(rx.borrow().lobby.is_some());
        assert!(rx.borrow().champion_id.is_none());
    }

    #[test]
    fn a_champion_in_both_slots_is_listed_once_and_empty_slots_are_skipped() {
        let raw = r#"{ "localMember": { "playerSlots": [
            { "championId": 238, "skinId": 238000 },
            { "championId": 0, "skinId": 0 },
            { "championId": 238, "skinId": 238012 } ] } }"#;
        let lobby: LobbySession = serde_json::from_str(raw).expect("valid lobby");
        assert_eq!(
            lobby.chosen_slots(),
            vec![LobbySlot {
                champion_id: 238,
                skin_id: 238_000
            }]
        );
    }

    #[test]
    fn leaving_the_lobby_clears_the_picks() {
        let lobby: LobbySession = serde_json::from_str(SWIFTPLAY).expect("valid lobby");
        let (tx, rx) = new_state_channel();
        set_phase(&tx, GamePhase::Lobby);
        apply_lobby_to_state(&tx, Some(&lobby));
        apply_lobby_to_state(&tx, None);
        assert!(rx.borrow().lobby.is_none());
        assert!(rx.borrow().queue.is_none());
        assert!(rx.borrow().champion_id.is_none());
    }
}
