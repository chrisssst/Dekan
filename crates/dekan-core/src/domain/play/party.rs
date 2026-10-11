use serde::{Deserialize, Serialize};

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
        ]
    }

    #[test]
    fn test_only_real_teammates_on_their_real_champion_are_accepted() {
        let peers = vec![
            peer(1, "friend", 103, 103_015),
            peer(2, "me", 238, 238_001),
            peer(3, "stranger", 99, 99_002),
            peer(4, "other", 103, 103_010),
            peer(5, "other", 1, 1_000),
        ];
        let (accepted, rejected) = verified_party_skins(&peers, &team(), Some("me"));

        assert_eq!(accepted, vec![(103, 103_015)]);
        assert!(rejected.contains(&(2, PeerRejection::Ourselves)));
        assert!(rejected.contains(&(3, PeerRejection::NotATeammate)));
        assert!(rejected.contains(&(4, PeerRejection::ChampionMismatch { real: 1 })));
        assert!(rejected.contains(&(5, PeerRejection::BaseSkin)));
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
}
