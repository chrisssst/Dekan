use std::time::Duration;

use dekan_core::phase::GamePhase;
use dekan_core::state::{StateSender, set_phase};
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use crate::champ_select::{ChampSelectSession, apply_session_to_state};
use crate::error::LcuError;

pub const TOPIC_GAMEFLOW: &str = "OnJsonApiEvent_lol-gameflow_v1_gameflow-phase";

pub const TOPIC_CHAMP_SELECT: &str = "OnJsonApiEvent_lol-champ-select_v1_session";

pub const TOPIC_LOBBY: &str = "OnJsonApiEvent_lol-lobby_v2_lobby";

#[derive(Debug, Clone)]
pub struct BackoffManager {
    initial: Duration,
    max: Duration,
    multiplier: f64,
    current: Duration,
    attempts: usize,
}

impl Default for BackoffManager {
    fn default() -> Self {
        Self {
            initial: Duration::from_millis(500),
            max: Duration::from_secs(15),
            multiplier: 2.0,
            current: Duration::from_millis(500),
            attempts: 0,
        }
    }
}

impl BackoffManager {
    pub fn next_delay(&mut self) -> Duration {
        self.attempts += 1;
        let delay = self.current;

        let next_ms = (self.current.as_millis() as f64 * self.multiplier) as u64;
        self.current = Duration::from_millis(next_ms).min(self.max);

        delay
    }

    pub fn reset(&mut self) {
        self.current = self.initial;
        self.attempts = 0;
    }

    #[must_use]
    pub fn attempts(&self) -> usize {
        self.attempts
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LcuEventPayload {
    pub uri: String,

    pub event_type: String,

    pub data: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LcuEvent {
    pub opcode: i64,

    pub topic: String,

    pub payload: LcuEventPayload,
}

impl LcuEvent {
    pub fn parse(text: &str) -> Result<Option<Self>, LcuError> {
        let trimmed = text.trim();
        if !trimmed.starts_with('[') || !trimmed.ends_with(']') {
            return Ok(None);
        }

        let array: Vec<serde_json::Value> = serde_json::from_str(trimmed)?;
        if array.len() != 3 {
            return Ok(None);
        }

        let opcode = array[0]
            .as_i64()
            .ok_or_else(|| LcuError::Parse("opcode is not an integer".into()))?;

        if opcode != 8 {
            return Ok(None);
        }

        let topic = array[1]
            .as_str()
            .ok_or_else(|| LcuError::Parse("topic is not a string".into()))?
            .to_string();

        let payload: LcuEventPayload = serde_json::from_value(array[2].clone())?;

        Ok(Some(Self {
            opcode,
            topic,
            payload,
        }))
    }
}

#[must_use]
pub fn make_subscribe_frame(topic: &str) -> String {
    format!(r#"[5,"{topic}"]"#)
}

pub fn dispatch_event_to_state(state_tx: &StateSender, event: &LcuEvent) {
    match event.topic.as_str() {
        TOPIC_GAMEFLOW => {
            if let Some(phase_str) = event.payload.data.as_str() {
                let phase = GamePhase::from_lcu(phase_str).unwrap_or_else(|| {
                    warn!(
                        raw = %phase_str,
                        "Unmapped LCU gameflow phase; treating it as None"
                    );
                    GamePhase::None
                });
                info!(phase = ?phase, raw = %phase_str, "LCU gameflow phase transition");
                set_phase(state_tx, phase);
            }
        }
        TOPIC_CHAMP_SELECT => {
            match serde_json::from_value::<ChampSelectSession>(event.payload.data.clone()) {
                Ok(session) => {
                    debug!(
                        cell_id = session.local_player_cell_id,
                        team = session.my_team.len(),
                        finalization = session.is_finalization(),
                        custom_game = session.is_custom_game,
                        "Champ-select session update received"
                    );
                    apply_session_to_state(state_tx, &session);
                }

                Err(e) => {
                    warn!(error = %e, "Could not read the champ-select session from the event");
                }
            }
        }
        TOPIC_LOBBY => {
            if event.payload.data.is_null() || event.payload.event_type == "Delete" {
                crate::lobby::apply_lobby_to_state(state_tx, None);
                return;
            }
            match serde_json::from_value::<crate::lobby::LobbySession>(event.payload.data.clone()) {
                Ok(lobby) => crate::lobby::apply_lobby_to_state(state_tx, Some(&lobby)),
                Err(e) => warn!(error = %e, "Could not read the lobby from the event"),
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dekan_core::state::new_state_channel;

    #[test]
    fn test_backoff_growth_and_reset() {
        let mut backoff = BackoffManager::default();

        let d1 = backoff.next_delay();
        assert_eq!(d1, Duration::from_millis(500));

        let d2 = backoff.next_delay();
        assert_eq!(d2, Duration::from_millis(1000));

        let d3 = backoff.next_delay();
        assert_eq!(d3, Duration::from_millis(2000));

        backoff.reset();
        assert_eq!(backoff.next_delay(), Duration::from_millis(500));
        assert_eq!(backoff.attempts(), 1);
    }

    #[test]
    fn test_parse_lcu_event_frame() {
        let raw = r#"[8, "OnJsonApiEvent_lol-gameflow_v1_gameflow-phase", {"uri": "/lol-gameflow/v1/gameflow-phase", "eventType": "Update", "data": "ChampSelect"}]"#;
        let event = LcuEvent::parse(raw)
            .expect("parse frame")
            .expect("event exists");

        assert_eq!(event.opcode, 8);
        assert_eq!(event.topic, TOPIC_GAMEFLOW);
        assert_eq!(event.payload.data, "ChampSelect");

        let (tx, rx) = new_state_channel();
        dispatch_event_to_state(&tx, &event);
        assert_eq!(rx.borrow().phase, GamePhase::ChampSelect);
    }

    #[test]
    fn lobby_events_open_and_close_the_lobby_picks() {
        let (tx, rx) = new_state_channel();
        set_phase(&tx, GamePhase::Lobby);
        let created = r#"[8, "OnJsonApiEvent_lol-lobby_v2_lobby", {"uri": "/lol-lobby/v2/lobby", "eventType": "Update",
            "data": {"gameConfig": {"queueId": 480, "gameMode": "SWIFTPLAY", "mapId": 11},
                     "localMember": {"playerSlots": [{"championId": 238, "skinId": 238000}]}}}]"#;
        let event = LcuEvent::parse(created).expect("frame").expect("event");
        dispatch_event_to_state(&tx, &event);
        assert_eq!(rx.borrow().champion_id, Some(238));

        let deleted = r#"[8, "OnJsonApiEvent_lol-lobby_v2_lobby", {"uri": "/lol-lobby/v2/lobby", "eventType": "Delete", "data": null}]"#;
        let event = LcuEvent::parse(deleted).expect("frame").expect("event");
        dispatch_event_to_state(&tx, &event);
        assert!(rx.borrow().lobby.is_none());
    }

    #[test]
    fn test_make_subscribe_frame() {
        let frame = make_subscribe_frame(TOPIC_GAMEFLOW);
        assert_eq!(
            frame,
            r#"[5,"OnJsonApiEvent_lol-gameflow_v1_gameflow-phase"]"#
        );
    }
}
