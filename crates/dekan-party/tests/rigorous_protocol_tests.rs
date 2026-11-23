use dekan_core::identity::IdentityLoadout;
use dekan_party::crypto::RoomCipher;
use dekan_party::protocol::{Announcement, RelayMember, open_member};

#[test]
fn crypto_detects_every_single_bit_flip_and_wrong_key() {
    let cipher = RoomCipher::new(&[0x5Au8; 32]);
    let other_cipher = RoomCipher::new(&[0xA5u8; 32]);

    let ann = Announcement {
        member_id: 42,
        puuid: "player-uuid-42".into(),
        champion_id: 238,
        skin_id: 238_015,
        chroma_id: Some(238_020),
        identity: Some(IdentityLoadout {
            icon_id: Some(29),
            border: Some("CHALLENGER".into()),
            banner: Some("/lol-game-data/assets/v1/regalia/banner.png".into()),
            title: Some("Bot Diff".into()),
            tokens: vec!["101000:CHALLENGER".into()],
        }),
    };

    let blob = ann.seal(&cipher).expect("seal");
    assert!(other_cipher.open(&blob).is_err());

    let mut raw_json = serde_json::to_value(&blob).expect("blob json");
    let ct_b64 = raw_json["c"].as_str().expect("ciphertext").to_owned();
    assert!(!ct_b64.is_empty());

    let mut chars: Vec<u8> = ct_b64.into_bytes();
    for pos in 0..chars.len().min(48) {
        let orig = chars[pos];
        chars[pos] = if orig == b'A' { b'B' } else { b'A' };
        raw_json["c"] = serde_json::Value::String(String::from_utf8(chars.clone()).expect("utf8"));
        if let Ok(mutated) = serde_json::from_value(raw_json.clone()) {
            assert!(cipher.open(&mutated).is_err());
        }
        chars[pos] = orig;
    }
}

#[test]
fn lobby_identity_only_announcement_opens_and_rejects_hostile_payloads() {
    let cipher = RoomCipher::new(&[7u8; 32]);

    let lobby_ann = Announcement {
        member_id: 11,
        puuid: "friend-11".into(),
        champion_id: 0,
        skin_id: 0,
        chroma_id: None,
        identity: Some(IdentityLoadout {
            icon_id: Some(502),
            border: Some("GRANDMASTER".into()),
            banner: Some("/lol-game-data/assets/v1/regalia/gm.png".into()),
            title: Some("Rift Legend".into()),
            tokens: vec![
                "/lol-game-data/assets/assets/challenges/t1.png".into(),
                "/lol-game-data/assets/assets/challenges/t2.png".into(),
            ],
        }),
    };
    let blob = lobby_ann.seal(&cipher).expect("seal");
    let member = RelayMember {
        summoner_id: serde_json::json!(11),
        skin: Some(serde_json::to_value(&blob).expect("json")),
    };
    let peer = open_member(&member, &cipher, 1)
        .expect("valid")
        .expect("peer present");
    assert_eq!(peer.champion_id, 0);
    assert_eq!(peer.identity, lobby_ann.identity);

    let hostile_banners = [
        "javascript:alert(1)",
        "/lol-game-data/assets/a.png);color:red",
        "/lol-game-data/assets/../../etc/passwd",
        "https://evil.example/a.png",
    ];
    for bad_banner in hostile_banners {
        let bad = Announcement {
            member_id: 11,
            puuid: "friend-11".into(),
            champion_id: 0,
            skin_id: 0,
            chroma_id: None,
            identity: Some(IdentityLoadout {
                banner: Some(bad_banner.into()),
                ..IdentityLoadout::default()
            }),
        };
        let bad_blob = bad.seal(&cipher).expect("seal");
        let bad_member = RelayMember {
            summoner_id: serde_json::json!(11),
            skin: Some(serde_json::to_value(&bad_blob).expect("json")),
        };
        assert!(open_member(&bad_member, &cipher, 1).is_err());
    }
}
