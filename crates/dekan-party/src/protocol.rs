use dekan_core::party::PartyPeer;
use serde::{Deserialize, Serialize};

use crate::crypto::{RoomCipher, SealedBlob};
use crate::error::PartyError;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ClientMessage {
    Join {
        summoner_id: u64,
        summoner_name: String,
    },
    Skin {
        skin: Option<SealedBlob>,
    },
    Leave,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RelayMember {
    #[serde(default)]
    pub summoner_id: serde_json::Value,
    #[serde(default)]
    pub skin: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum RelayMessage {
    Members {
        #[serde(default)]
        members: Vec<RelayMember>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Announcement {
    pub member_id: u64,
    pub puuid: String,
    pub champion_id: u32,
    pub skin_id: u32,
    pub chroma_id: Option<u32>,
}

const MAX_PUUID_LEN: usize = 100;

impl Announcement {
    pub fn validate(&self) -> Result<(), PartyError> {
        let bad = |why: &str| Err(PartyError::InvalidAnnouncement(why.into()));
        if self.puuid.is_empty()
            || self.puuid.len() > MAX_PUUID_LEN
            || !self
                .puuid
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return bad("puuid");
        }

        if self.champion_id == 0 || self.champion_id >= 70_000 {
            return bad("champion");
        }
        if self.skin_id / 1000 != self.champion_id {
            return bad("skin does not belong to the champion");
        }
        if self
            .chroma_id
            .is_some_and(|chroma| chroma / 1000 != self.champion_id)
        {
            return bad("chroma does not belong to the champion");
        }
        Ok(())
    }

    pub fn seal(&self, cipher: &RoomCipher) -> Result<SealedBlob, PartyError> {
        cipher.seal(&serde_json::to_vec(self)?)
    }
}

pub fn open_member(
    member: &RelayMember,
    cipher: &RoomCipher,
    own_member: u64,
) -> Result<Option<PartyPeer>, PartyError> {
    let Some(member_id) = member.summoner_id.as_u64().filter(|id| *id != 0) else {
        return Err(PartyError::InvalidAnnouncement("member id".into()));
    };
    if member_id == own_member {
        return Ok(None);
    }
    let Some(skin) = member.skin.as_ref().filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    let blob: SealedBlob = serde_json::from_value(skin.clone())?;
    let plaintext = cipher.open(&blob)?;
    let announcement: Announcement = serde_json::from_slice(&plaintext)?;
    if announcement.member_id != member_id {
        return Err(PartyError::InvalidAnnouncement(
            "announcement is bound to another member (replayed)".into(),
        ));
    }
    announcement.validate()?;
    Ok(Some(PartyPeer {
        member_id,
        puuid: announcement.puuid,
        champion_id: announcement.champion_id,
        skin_id: announcement.skin_id,
        chroma_id: announcement.chroma_id,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn announcement(member_id: u64) -> Announcement {
        Announcement {
            member_id,
            puuid: "abc-123".into(),
            champion_id: 103,
            skin_id: 103_015,
            chroma_id: Some(103_020),
        }
    }

    fn member(member_id: u64, blob: &SealedBlob) -> RelayMember {
        RelayMember {
            summoner_id: serde_json::json!(member_id),
            skin: Some(serde_json::to_value(blob).expect("json")),
        }
    }

    #[test]
    fn test_wire_matches_the_relay_protocol() {
        let join = serde_json::to_value(ClientMessage::Join {
            summoner_id: 5,
            summoner_name: String::new(),
        })
        .expect("json");
        assert_eq!(join["type"], "join");
        assert_eq!(join["summoner_id"], 5);
        let clear = serde_json::to_value(ClientMessage::Skin { skin: None }).expect("json");
        assert_eq!(clear, serde_json::json!({"type": "skin", "skin": null}));

        let members: RelayMessage = serde_json::from_str(
            r#"{"type":"members","members":[{"summoner_id":5,"summoner_name":"Unknown"}]}"#,
        )
        .expect("members parse");
        let RelayMessage::Members { members } = members;
        assert_eq!(members.len(), 1);
    }

    #[test]
    fn test_a_valid_member_opens_into_a_peer() {
        let cipher = RoomCipher::new(&[3u8; 32]);
        let blob = announcement(9).seal(&cipher).expect("seal");
        let peer = open_member(&member(9, &blob), &cipher, 1)
            .expect("valid")
            .expect("has a skin");
        assert_eq!(peer.champion_id, 103);
        assert_eq!(peer.entry_id(), 103_020);

        assert_eq!(
            open_member(&member(1, &blob), &cipher, 1).expect("ok"),
            None,
            "our own echo is ignored"
        );
    }

    #[test]
    fn test_replays_and_invalid_fields_are_refused() {
        let cipher = RoomCipher::new(&[3u8; 32]);

        let blob = announcement(9).seal(&cipher).expect("seal");
        assert!(open_member(&member(7, &blob), &cipher, 1).is_err());

        let mut foreign_skin = announcement(9);
        foreign_skin.skin_id = 1_001;
        let blob = foreign_skin.seal(&cipher).expect("seal");
        assert!(open_member(&member(9, &blob), &cipher, 1).is_err());

        let mut classic = announcement(9);
        classic.champion_id = 60_103;
        classic.skin_id = 60_103_001;
        classic.chroma_id = None;
        assert!(
            classic.validate().is_ok(),
            "party applies to Rift Classic champions too"
        );

        let mut out_of_range = announcement(9);
        out_of_range.champion_id = 70_001;
        out_of_range.skin_id = 70_001_001;
        assert!(
            out_of_range.validate().is_err(),
            "champion IDs >= 70_000 are refused"
        );

        let mut hostile = announcement(9);
        hostile.puuid = "../../evil".into();
        assert!(hostile.validate().is_err());

        let unreadable = RelayMember {
            summoner_id: serde_json::json!(9),
            skin: Some(serde_json::json!({"champion_id": 103, "skin_id": 103015})),
        };
        assert!(
            open_member(&unreadable, &cipher, 1).is_err(),
            "a cleartext skin from another client is not accepted"
        );
    }
}
