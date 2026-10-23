use serde::{Deserialize, Serialize};

use crate::phase::GamePhase;
use crate::selection::{ChampionId, ChromaId, SkinId};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamMember {
    pub puuid: String,
    pub champion_id: ChampionId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartyPeer {
    pub member_id: u64,
    pub puuid: String,
    pub champion_id: ChampionId,
    pub skin_id: SkinId,
    pub chroma_id: Option<ChromaId>,
}

impl PartyPeer {
    #[must_use]
    pub fn entry_id(&self) -> u32 {
        self.chroma_id.unwrap_or(self.skin_id)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PartyStatus {
    #[default]
    Off,

    Unavailable {
        reason: String,
    },

    Connecting,

    Connected {
        members: usize,
    },

    Error {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeerRejection {
    Ourselves,

    NotATeammate,

    ChampionMismatch { real: ChampionId },

    BaseSkin,

    Disputed,
}

pub type VerifiedParty = (Vec<(ChampionId, u32)>, Vec<(u64, PeerRejection)>);

#[must_use]
pub fn verified_party_skins(
    peers: &[PartyPeer],
    team: &[TeamMember],
    local_puuid: Option<&str>,
) -> VerifiedParty {
    let mut accepted: Vec<(ChampionId, u32)> = Vec::new();
    let mut rejected = Vec::new();

    for peer in peers {
        if local_puuid.is_some_and(|me| me == peer.puuid) {
            rejected.push((peer.member_id, PeerRejection::Ourselves));
            continue;
        }
        if peers
            .iter()
            .any(|other| other.member_id != peer.member_id && other.puuid == peer.puuid)
        {
            rejected.push((peer.member_id, PeerRejection::Disputed));
            continue;
        }
        let Some(teammate) = team.iter().find(|t| t.puuid == peer.puuid) else {
            rejected.push((peer.member_id, PeerRejection::NotATeammate));
            continue;
        };
        if teammate.champion_id != peer.champion_id {
            rejected.push((
                peer.member_id,
                PeerRejection::ChampionMismatch {
                    real: teammate.champion_id,
                },
            ));
            continue;
        }
        let entry = peer.entry_id();
        if entry == peer.champion_id * 1000 {
            rejected.push((peer.member_id, PeerRejection::BaseSkin));
            continue;
        }
        if !accepted
            .iter()
            .any(|(champion, _)| *champion == peer.champion_id)
        {
            accepted.push((peer.champion_id, entry));
        }
    }

    accepted.sort_unstable();
    (accepted, rejected)
}

#[must_use]
pub fn party_fingerprint(accepted: &[(ChampionId, u32)]) -> u64 {
    if accepted.is_empty() {
        return 0;
    }
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for (champion, entry) in accepted {
        for byte in champion
            .to_le_bytes()
            .into_iter()
            .chain(entry.to_le_bytes())
        {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    hash.max(1)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PartySummary {
    pub members: usize,
    pub teammate_skins: usize,
}

#[must_use]
pub fn party_summary(
    status: &PartyStatus,
    peers: &[PartyPeer],
    team: &[TeamMember],
    local_puuid: Option<&str>,
) -> Option<PartySummary> {
    let PartyStatus::Connected { members } = status else {
        return None;
    };
    Some(PartySummary {
        members: *members,
        teammate_skins: verified_party_skins(peers, team, local_puuid).0.len(),
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LobbyParty {
    pub party_id: String,
    pub members: usize,
}

#[must_use]
pub fn lobby_room(
    current: Option<&str>,
    lobby: Option<&LobbyParty>,
    phase: GamePhase,
) -> Option<String> {
    match lobby {
        Some(lobby) if lobby.members >= 2 && !lobby.party_id.is_empty() => {
            Some(lobby.party_id.clone())
        }
        Some(_) => None,
        None if phase.is_between_matches() => None,
        None => current.map(str::to_owned),
    }
}

#[must_use]
pub fn may_ask_to_share(phase: GamePhase) -> bool {
    matches!(phase, GamePhase::Lobby | GamePhase::EndOfGame)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer(member_id: u64, puuid: &str, champion_id: u32, skin_id: u32) -> PartyPeer {
        PartyPeer {
            member_id,
            puuid: puuid.into(),
            champion_id,
            skin_id,
            chroma_id: None,
        }
    }

    fn team() -> Vec<TeamMember> {
        vec![
            TeamMember {
                puuid: "me".into(),
                champion_id: 238,
            },
            TeamMember {
                puuid: "friend".into(),
                champion_id: 103,
            },
            TeamMember {
                puuid: "other".into(),
                champion_id: 1,
            },
            TeamMember {
                puuid: "plain".into(),
                champion_id: 2,
            },
        ]
    }

    #[test]
    fn test_only_real_teammates_on_their_real_champion_are_accepted() {
        let peers = vec![
            peer(1, "friend", 103, 103_015),
            peer(2, "me", 238, 238_001),
            peer(3, "stranger", 99, 99_002),
            peer(4, "other", 103, 103_010),
            peer(5, "plain", 2, 2_000),
        ];
        let (accepted, rejected) = verified_party_skins(&peers, &team(), Some("me"));

        assert_eq!(accepted, vec![(103, 103_015)]);
        assert!(rejected.contains(&(2, PeerRejection::Ourselves)));
        assert!(rejected.contains(&(3, PeerRejection::NotATeammate)));
        assert!(rejected.contains(&(4, PeerRejection::ChampionMismatch { real: 1 })));
        assert!(rejected.contains(&(5, PeerRejection::BaseSkin)));
    }

    #[test]
    fn two_members_claiming_one_teammate_are_both_refused() {
        let peers = vec![
            peer(1, "friend", 103, 103_015),
            peer(7, "friend", 103, 103_002),
        ];
        let (accepted, rejected) = verified_party_skins(&peers, &team(), Some("me"));

        assert!(accepted.is_empty());
        assert!(rejected.contains(&(1, PeerRejection::Disputed)));
        assert!(rejected.contains(&(7, PeerRejection::Disputed)));
    }

    #[test]
    fn a_dispute_refuses_only_the_claimants_and_spares_the_rest() {
        let peers = vec![
            peer(1, "friend", 103, 103_015),
            peer(7, "friend", 103, 103_002),
            peer(5, "other", 1, 1_002),
            peer(2, "me", 238, 238_001),
            peer(8, "me", 238, 238_003),
        ];
        let (accepted, rejected) = verified_party_skins(&peers, &team(), Some("me"));

        assert_eq!(accepted, vec![(1, 1_002)]);
        assert!(rejected.contains(&(1, PeerRejection::Disputed)));
        assert!(rejected.contains(&(7, PeerRejection::Disputed)));
        assert!(rejected.contains(&(2, PeerRejection::Ourselves)));
        assert!(rejected.contains(&(8, PeerRejection::Ourselves)));
    }

    #[test]
    fn a_member_repeating_its_own_claim_is_not_a_dispute() {
        let peers = vec![
            peer(1, "friend", 103, 103_015),
            peer(1, "friend", 103, 103_015),
        ];
        let (accepted, rejected) = verified_party_skins(&peers, &team(), Some("me"));

        assert_eq!(accepted, vec![(103, 103_015)]);
        assert!(rejected.is_empty());
    }

    #[test]
    fn test_chroma_is_the_injected_entry_and_the_fingerprint_is_stable() {
        let mut with_chroma = peer(1, "friend", 103, 103_015);
        with_chroma.chroma_id = Some(103_020);
        let (accepted, _) = verified_party_skins(&[with_chroma], &team(), Some("me"));
        assert_eq!(accepted, vec![(103, 103_020)]);

        assert_eq!(party_fingerprint(&[]), 0);
        let fp = party_fingerprint(&accepted);
        assert_ne!(fp, 0);
        assert_eq!(fp, party_fingerprint(&accepted.clone()));
        assert_ne!(fp, party_fingerprint(&[(103, 103_015)]));
    }

    #[test]
    fn test_party_summary_counts_only_verified_teammate_skins_while_connected() {
        let peers = vec![
            peer(1, "friend", 103, 103_015),
            peer(3, "stranger", 99, 99_002),
        ];
        let connected = PartyStatus::Connected { members: 3 };
        assert_eq!(
            party_summary(&connected, &peers, &team(), Some("me")),
            Some(PartySummary {
                members: 3,
                teammate_skins: 1
            })
        );
        assert_eq!(
            party_summary(&PartyStatus::Connecting, &peers, &team(), Some("me")),
            None
        );
    }

    fn lobby(party_id: &str, members: usize) -> LobbyParty {
        LobbyParty {
            party_id: party_id.into(),
            members,
        }
    }

    #[test]
    fn a_lobby_with_friends_opens_its_room_and_a_solo_lobby_does_not() {
        let duo = lobby("p-1", 2);
        assert_eq!(
            lobby_room(None, Some(&duo), GamePhase::Lobby),
            Some("p-1".into())
        );
        assert_eq!(
            lobby_room(None, Some(&lobby("p-1", 1)), GamePhase::Lobby),
            None
        );
        assert_eq!(
            lobby_room(None, Some(&lobby("", 3)), GamePhase::Lobby),
            None
        );
    }

    #[test]
    fn the_room_follows_the_lobby_and_closes_when_friends_leave() {
        assert_eq!(
            lobby_room(Some("p-1"), Some(&lobby("p-2", 3)), GamePhase::Lobby),
            Some("p-2".into())
        );
        assert_eq!(
            lobby_room(Some("p-1"), Some(&lobby("p-1", 1)), GamePhase::Lobby),
            None
        );
    }

    #[test]
    fn the_room_outlives_the_lobby_during_the_match_but_not_after_it() {
        for phase in [
            GamePhase::Matchmaking,
            GamePhase::ChampSelect,
            GamePhase::InProgress,
            GamePhase::Reconnect,
            GamePhase::WaitingForStats,
        ] {
            assert_eq!(lobby_room(Some("p-1"), None, phase), Some("p-1".into()));
        }
        for phase in [GamePhase::None, GamePhase::Lobby, GamePhase::EndOfGame] {
            assert_eq!(lobby_room(Some("p-1"), None, phase), None);
        }
        assert_eq!(lobby_room(None, None, GamePhase::InProgress), None);
    }

    #[test]
    fn the_invite_never_interrupts_queue_pop_champion_select_or_the_match() {
        assert!(may_ask_to_share(GamePhase::Lobby));
        assert!(may_ask_to_share(GamePhase::EndOfGame));
        for phase in [
            GamePhase::Matchmaking,
            GamePhase::ReadyCheck,
            GamePhase::ChampSelect,
            GamePhase::Finalization,
            GamePhase::InProgress,
            GamePhase::Reconnect,
        ] {
            assert!(!may_ask_to_share(phase));
        }
    }
}
