use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "PascalCase")]
pub enum GamePhase {
    #[default]
    None,

    Lobby,

    Matchmaking,

    ReadyCheck,

    ChampSelect,

    Finalization,

    GameStart,

    InProgress,

    Reconnect,

    WaitingForStats,

    EndOfGame,

    CheckedIntoTournament,

    FailedToLaunch,

    PreEndOfGame,

    TerminatedInError,
}

impl GamePhase {
    #[must_use]
    pub fn from_lcu(raw: &str) -> Option<Self> {
        Some(match raw {
            "None" => Self::None,
            "Lobby" => Self::Lobby,
            "Matchmaking" => Self::Matchmaking,
            "CheckedIntoTournament" => Self::CheckedIntoTournament,
            "ReadyCheck" => Self::ReadyCheck,
            "ChampSelect" => Self::ChampSelect,
            "GameStart" => Self::GameStart,
            "FailedToLaunch" => Self::FailedToLaunch,
            "InProgress" => Self::InProgress,
            "Reconnect" => Self::Reconnect,
            "WaitingForStats" => Self::WaitingForStats,
            "PreEndOfGame" => Self::PreEndOfGame,
            "EndOfGame" => Self::EndOfGame,
            "TerminatedInError" => Self::TerminatedInError,
            _ => return None,
        })
    }

    #[must_use]
    pub fn is_between_matches(self) -> bool {
        matches!(
            self,
            Self::None
                | Self::Lobby
                | Self::EndOfGame
                | Self::FailedToLaunch
                | Self::TerminatedInError
        )
    }

    #[must_use]
    pub fn is_champ_select(self) -> bool {
        matches!(self, Self::ChampSelect | Self::Finalization)
    }

    #[must_use]
    pub fn is_in_game(self) -> bool {
        matches!(self, Self::GameStart | Self::InProgress | Self::Reconnect)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lcu_phase_strings_map_and_unknown_ones_are_reported_as_unknown() {
        assert_eq!(GamePhase::from_lcu("None"), Some(GamePhase::None));
        assert_eq!(
            GamePhase::from_lcu("TerminatedInError"),
            Some(GamePhase::TerminatedInError)
        );
        assert_eq!(
            GamePhase::from_lcu("ChampSelect"),
            Some(GamePhase::ChampSelect)
        );
        assert_eq!(
            GamePhase::from_lcu("Finalization"),
            None,
            "not an LCU phase: derived from the champ-select timer"
        );
        assert_eq!(GamePhase::from_lcu("SomethingNew"), None);
        assert!(GamePhase::FailedToLaunch.is_between_matches());
        assert!(GamePhase::TerminatedInError.is_between_matches());
        assert!(!GamePhase::WaitingForStats.is_between_matches());
        assert!(!GamePhase::TerminatedInError.is_in_game());
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum QueueType {
    Draft,

    BlindPick,

    Aram,

    Swiftplay,

    Arena,

    Rotating,

    ClassicRift,

    Unknown(i32),
}

impl QueueType {
    pub const CLASSIC_RIFT_QUEUE_ID: i32 = 3262;

    #[must_use]
    pub fn from_queue_id(id: i32) -> Self {
        match id {
            400 | 420 | 440 => Self::Draft,
            430 | 460 => Self::BlindPick,
            450 => Self::Aram,
            480 => Self::Swiftplay,
            1700 => Self::Arena,
            3262 => Self::ClassicRift,
            _ => Self::Unknown(id),
        }
    }

    #[must_use]
    pub fn is_classic_rift(self) -> bool {
        matches!(self, Self::ClassicRift)
    }
}
