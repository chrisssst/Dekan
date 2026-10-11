use dekan_core::selection::{ChampionId, SkinId};
use serde::Deserialize;
use tracing::{debug, warn};

use crate::client::LcuClient;
use crate::error::LcuError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionSource {
    ChampSelect,

    GameflowSession,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveSelection {
    pub champion_id: ChampionId,
    pub skin_id: SkinId,
    pub source: SelectionSource,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentSummoner {
    #[serde(default)]
    pub puuid: String,
    #[serde(default)]
    pub summoner_id: u64,
    #[serde(default)]
    pub internal_name: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerChampionSelection {
    #[serde(default)]
    pub champion_id: u32,

    #[serde(default)]
    pub selected_skin_index: u32,

    #[serde(default)]
    pub puuid: String,

    #[serde(default)]
    pub summoner_internal_name: String,
    #[serde(default)]
    pub summoner_id: Option<u64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameflowGameData {
    #[serde(default)]
    pub player_champion_selections: Vec<PlayerChampionSelection>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameflowSession {
    #[serde(default)]
    pub game_data: GameflowGameData,
}

impl GameflowSession {
    #[must_use]
    pub fn local_selection(
        &self,
        summoner: Option<&CurrentSummoner>,
        known_champion: Option<ChampionId>,
    ) -> Option<&PlayerChampionSelection> {
        if let Some(me) = summoner {
            let by_identity = self.game_data.player_champion_selections.iter().find(|p| {
                (!me.puuid.is_empty() && p.puuid == me.puuid)
                    || (me.summoner_id != 0 && p.summoner_id == Some(me.summoner_id))
                    || (!me.internal_name.is_empty()
                        && p.summoner_internal_name == me.internal_name)
            });
            if by_identity.is_some() {
                return by_identity;
            }
        }

        let champion = known_champion?;
        let mut matching = self
            .game_data
            .player_champion_selections
            .iter()
            .filter(|p| p.champion_id == champion);
        let first = matching.next()?;
        if matching.next().is_some() {
            warn!(
                champion_id = champion,
                "Several players locked this champion and the summoner is unknown; refusing to guess"
            );
            return None;
        }
        Some(first)
    }
}

pub async fn resolve_live_selection(
    client: &LcuClient,
    known_champion: Option<ChampionId>,
) -> Result<LiveSelection, LcuError> {
    match client.get_champ_select_session().await {
        Ok(session) => match session.local_selection() {
            Ok((champion_id, skin_id)) => {
                debug!(
                    champion_id,
                    skin_id, "Selection re-read from the champ select session"
                );
                return Ok(LiveSelection {
                    champion_id,
                    skin_id,
                    source: SelectionSource::ChampSelect,
                });
            }
            Err(e) => debug!(error = %e, "Champ select session has no local selection"),
        },
        Err(e) => debug!(error = %e, "Champ select session unavailable; falling back to gameflow"),
    }

    let summoner = match client.get_current_summoner().await {
        Ok(summoner) => Some(summoner),
        Err(e) => {
            debug!(error = %e, "Current summoner unavailable for the gameflow re-read");
            None
        }
    };
    let session = client.get_gameflow_session().await?;

    let selection = session
        .local_selection(summoner.as_ref(), known_champion)
        .ok_or_else(|| {
            LcuError::Parse("no local player selection in the gameflow session".into())
        })?;

    debug!(
        champion_id = selection.champion_id,
        skin_id = selection.selected_skin_index,
        "Selection re-read from the gameflow session"
    );

    Ok(LiveSelection {
        champion_id: selection.champion_id,
        skin_id: selection.selected_skin_index,
        source: SelectionSource::GameflowSession,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SESSION: &str = r#"{
        "gameData": {
            "playerChampionSelections": [
                { "championId": 103, "puuid": "aaaa-1111", "selectedSkinIndex": 103001, "spell1Id": 4, "spell2Id": 21 },
                { "championId": 1, "puuid": "bbbb-2222", "selectedSkinIndex": 1042, "spell1Id": 4, "spell2Id": 14 }
            ]
        }
    }"#;

    fn session() -> GameflowSession {
        serde_json::from_str(SESSION).expect("valid gameflow session")
    }

    #[test]
    fn test_matches_local_player_by_puuid() {
        let me = CurrentSummoner {
            puuid: "bbbb-2222".into(),
            summoner_id: 0,
            internal_name: String::new(),
        };
        let pick = session().local_selection(Some(&me), None).cloned();
        let pick = pick.expect("local player must be found");
        assert_eq!(pick.champion_id, 1);
        assert_eq!(pick.selected_skin_index, 1042);
    }

    #[test]
    fn test_falls_back_to_the_known_champion_when_summoner_is_unknown() {
        let pick = session().local_selection(None, Some(1)).cloned();
        assert_eq!(pick.expect("fallback").selected_skin_index, 1042);
    }

    #[test]
    fn test_refuses_to_guess_when_two_players_locked_the_same_champion() {
        let raw = r#"{
            "gameData": {
                "playerChampionSelections": [
                    { "championId": 1, "puuid": "aaaa-1111", "selectedSkinIndex": 1001 },
                    { "championId": 1, "puuid": "bbbb-2222", "selectedSkinIndex": 1042 }
                ]
            }
        }"#;
        let session: GameflowSession = serde_json::from_str(raw).expect("valid");
        assert!(
            session.local_selection(None, Some(1)).is_none(),
            "picking either one would inject a teammate's skin half the time"
        );
    }

    #[test]
    fn test_empty_session_yields_no_selection() {
        let session: GameflowSession = serde_json::from_str("{}").expect("tolerates empty body");
        assert!(session.local_selection(None, Some(1)).is_none());
    }
}
