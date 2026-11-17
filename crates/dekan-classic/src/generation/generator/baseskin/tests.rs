use super::*;
use crate::gear_toggle::named;
use crate::sound_bank::{Object, Section, SoundBank, wwise_id};
use banks::{
    HIRC_ACTION, HIRC_EVENT, ParsedEventBank, decode_actions, encode_actions, sidecar_path,
};
use dekan_wad::prop::{FIELD_STRING, PropEntry, PropFile};

const FIELD_LIST: u8 = 0x80;

fn make_skin_bin(skin_name: &str, skel: &str, mesh: &str, tags: &[&str]) -> Vec<u8> {
    let mesh_struct = Value::Struct {
        kind: crate::vfx_markers::FIELD_EMBED,
        class: h("SkinMeshDataProperties"),
        fields: vec![
            named("skeleton", text(skel)),
            named("simpleSkin", text(mesh)),
        ],
    };
    let audio_struct = Value::Struct {
        kind: crate::vfx_markers::FIELD_EMBED,
        class: h("SkinAudioProperties"),
        fields: vec![named(
            "tagEventList",
            Value::List {
                kind: FIELD_LIST,
                element: FIELD_STRING,
                items: tags.iter().map(|t| text(t)).collect(),
            },
        )],
    };
    let body = tree::write_fields(&[
        named("championSkinName", text(skin_name)),
        named("skinMeshProperties", mesh_struct),
        named("skinAudioProperties", audio_struct),
    ])
    .expect("write fields");
    serialize_prop_file(&PropFile {
        version: 2,
        links: Vec::new(),
        entries: vec![PropEntry {
            class_hash: SKIN_DATA_CLASS,
            key_hash: h("Characters/TestChamp/Skins/Skin0"),
            body,
        }],
    })
    .expect("serialize")
}

fn make_test_bank(version: u32, bank_id: u32, objects: Vec<Object>) -> Vec<u8> {
    let mut bkhd = Vec::new();
    bkhd.extend_from_slice(&version.to_le_bytes());
    bkhd.extend_from_slice(&bank_id.to_le_bytes());
    bkhd.extend_from_slice(&0u32.to_le_bytes());
    bkhd.extend_from_slice(&1u32.to_le_bytes());

    let mut hirc = (objects.len() as u32).to_le_bytes().to_vec();
    for obj in objects {
        let size = (obj.payload.len() + 4) as u32;
        hirc.push(obj.kind);
        hirc.extend_from_slice(&size.to_le_bytes());
        hirc.extend_from_slice(&obj.id.to_le_bytes());
        hirc.extend_from_slice(&obj.payload);
    }

    SoundBank {
        sections: vec![
            Section {
                tag: *b"BKHD",
                data: bkhd,
            },
            Section {
                tag: *b"HIRC",
                data: hirc,
            },
        ],
    }
    .write()
    .expect("write bank")
}

#[test]
fn padding_extends_without_duplicating() {
    let dir = std::env::temp_dir().join(format!("dekan_baseskin_pad_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: fixture cleanup
    write_padded_asset(&dir, "assets/test.skl", b"abc".to_vec()).expect("write");
    let written = std::fs::read(dir.join("assets/test.skl")).expect("read");
    assert_eq!(written.len(), 3 + 16);
    assert!(written.ends_with(&MESH_PADDING));
    write_padded_asset(&dir, "assets/test.skl", written.clone()).expect("rewrite");
    let rewritten = std::fs::read(dir.join("assets/test.skl")).expect("read2");
    assert_eq!(rewritten.len(), 3 + 16);
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: fixture cleanup
}

#[test]
fn make_skin_bin_round_trips_mesh_and_name() {
    let bin = make_skin_bin("Yone", "base.skl", "base.skn", &["Yone", "YoneSkin10"]);
    let parsed = parse_prop_file(&bin).expect("parse");
    let fields = tree::parse_fields(&parsed.entries[0].body).expect("fields");
    assert_eq!(
        tree::field(&fields, h("championSkinName"))
            .and_then(raw_text)
            .as_deref(),
        Some("Yone")
    );
    let mesh = tree::field(&fields, h("skinMeshProperties"))
        .and_then(Value::fields)
        .expect("mesh");
    assert_eq!(
        tree::field(mesh, h("skeleton"))
            .and_then(raw_text)
            .as_deref(),
        Some("base.skl")
    );
}

#[test]
fn make_skin_bin_round_trips_mesh_and_audio_tags_and_sfx_bank() {
    let wad_dir = std::env::temp_dir().join(format!("dekan_audio_test_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&wad_dir); // ignore-ok: cleanup
    std::fs::create_dir_all(&wad_dir).expect("mkdir");

    let skin_sfx_rel = "ASSETS/Sounds/Neeko_Skin10_SFX_events.bnk";
    let base_sfx_rel = "ASSETS/Sounds/Neeko_Base_SFX_events.bnk";
    let skin_ev = wwise_id("Play_sfx_NeekoSkin10_Q");
    let alias_ev = wwise_id("Play_sfx_Neeko_Q");

    let skin_bank_bytes = make_test_bank(
        145,
        10,
        vec![
            Object {
                kind: HIRC_ACTION,
                id: 500,
                payload: vec![9, 8, 7],
            },
            Object {
                kind: HIRC_EVENT,
                id: skin_ev,
                payload: encode_actions(&[500], 145),
            },
        ],
    );
    let base_bank_bytes = make_test_bank(
        145,
        11,
        vec![Object {
            kind: HIRC_EVENT,
            id: alias_ev,
            payload: encode_actions(&[999], 145),
        }],
    );

    let write_local = |rel: &str, data: &[u8]| {
        let p = wad_dir.join(rel.to_ascii_lowercase());
        std::fs::create_dir_all(p.parent().expect("parent")).expect("mkdir p");
        std::fs::write(p, data).expect("write bnk");
    };
    write_local(skin_sfx_rel, &skin_bank_bytes);
    write_local(base_sfx_rel, &base_bank_bytes);

    let unit_base = Value::Struct {
        kind: crate::vfx_markers::FIELD_EMBED,
        class: h("BankUnit"),
        fields: vec![
            named("name", text("Neeko_Base_SFX")),
            named(
                "bankPath",
                Value::List {
                    kind: FIELD_LIST,
                    element: FIELD_STRING,
                    items: vec![text(base_sfx_rel)],
                },
            ),
            named(
                "events",
                Value::List {
                    kind: FIELD_LIST,
                    element: FIELD_STRING,
                    items: vec![text("Play_sfx_Neeko_Q")],
                },
            ),
        ],
    };
    let unit_skin = Value::Struct {
        kind: crate::vfx_markers::FIELD_EMBED,
        class: h("BankUnit"),
        fields: vec![
            named("name", text("Neeko_Skin10_SFX")),
            named(
                "bankPath",
                Value::List {
                    kind: FIELD_LIST,
                    element: FIELD_STRING,
                    items: vec![text(skin_sfx_rel)],
                },
            ),
            named(
                "events",
                Value::List {
                    kind: FIELD_LIST,
                    element: FIELD_STRING,
                    items: vec![text("Play_sfx_NeekoSkin10_Q")],
                },
            ),
        ],
    };
    let audio_struct = Value::Struct {
        kind: crate::vfx_markers::FIELD_EMBED,
        class: h("SkinAudioProperties"),
        fields: vec![
            named(
                "tagEventList",
                Value::List {
                    kind: FIELD_LIST,
                    element: FIELD_STRING,
                    items: vec![text("Neeko"), text("NeekoSkin10")],
                },
            ),
            named(
                "bankUnits",
                Value::List {
                    kind: FIELD_LIST,
                    element: crate::vfx_markers::FIELD_EMBED,
                    items: vec![unit_base, unit_skin],
                },
            ),
        ],
    };
    let mut fields = vec![named("skinAudioProperties", audio_struct)];

    let mut dummy_header = vec![0u8; 272];
    dummy_header[0] = b'R';
    dummy_header[1] = b'W';
    dummy_header[2] = 3;
    let dummy_wad_path = wad_dir.join("Neeko.wad.client");
    std::fs::write(&dummy_wad_path, &dummy_header).expect("stub wad");
    let wad = WadFile::open_toc_only(&dummy_wad_path).expect("open stub wad");

    let mut shipped = BTreeMap::new();
    let changed = audio::rebase_skin_audio(
        &wad,
        "neeko",
        &mut fields,
        Some(&["Neeko".to_string()]),
        &wad_dir,
        &mut shipped,
    )
    .expect("rebase audio");
    assert!(changed);

    let tags = units::string_items(
        tree::field(&fields, h("skinAudioProperties"))
            .and_then(Value::fields)
            .expect("audio"),
        "tagEventList",
    );
    assert_eq!(tags, vec!["Neeko".to_string()]);

    let patched_skin_bank = ParsedEventBank::parse_bytes(
        &std::fs::read(wad_dir.join(skin_sfx_rel.to_ascii_lowercase())).expect("read"),
    )
    .expect("parse skin bank");
    assert_eq!(patched_skin_bank.event_actions(alias_ev), Some(vec![500]));

    let patched_base_bank = ParsedEventBank::parse_bytes(
        &std::fs::read(wad_dir.join(base_sfx_rel.to_ascii_lowercase())).expect("read"),
    )
    .expect("parse base bank");
    assert!(!patched_base_bank.has_event(alias_ev));

    let _ = std::fs::remove_dir_all(&wad_dir); // ignore-ok: cleanup
}

#[test]
fn event_actions_codec_handles_legacy_and_var_uint_versions() {
    let actions = vec![0x1122_3344u32, 0xAABB_CCDDu32];
    let enc_legacy = encode_actions(&actions, 110);
    assert_eq!(decode_actions(&enc_legacy, 110), Some(actions.clone()));

    let enc_modern = encode_actions(&actions, 145);
    assert_eq!(decode_actions(&enc_modern, 145), Some(actions));

    let big: Vec<u32> = (0..130).collect();
    let enc_big = encode_actions(&big, 145);
    assert_eq!(enc_big[0..2], [0x81, 0x02]);
    assert_eq!(decode_actions(&enc_big, 145), Some(big));
}

#[test]
fn sidecar_extracts_matching_actions_and_aliases() {
    let src_id = wwise_id("Play_vo_NeekoSkin10_Attack");
    let alias_id = wwise_id("Play_vo_Neeko_Attack");
    let raw = make_test_bank(
        145,
        99,
        vec![
            Object {
                kind: HIRC_ACTION,
                id: 777,
                payload: vec![1, 2, 3, 4],
            },
            Object {
                kind: HIRC_EVENT,
                id: src_id,
                payload: encode_actions(&[777], 145),
            },
        ],
    );
    let parsed = ParsedEventBank::parse_bytes(&raw).expect("parse");
    let sidecar_bytes = parsed
        .extract_sidecar(
            wwise_id("Neeko_Skin10_VO_basetag_events"),
            &[(alias_id, src_id)],
        )
        .expect("extract");
    let sidecar = ParsedEventBank::parse_bytes(&sidecar_bytes).expect("parse sidecar");
    assert!(sidecar.has_event(alias_id));
    assert_eq!(sidecar.event_actions(alias_id), Some(vec![777]));
    assert_eq!(
        sidecar_path(
            "ASSETS/Sounds/Wwise2016/VO/pt_BR/Characters/Neeko/Skins/Skin10/Neeko_Skin10_VO_events.bnk"
        ),
        "ASSETS/Sounds/Wwise2016/VO/Characters/Neeko/Skins/Skin10/Neeko_Skin10_VO_basetag_events.bnk"
    );
}

#[test]
fn live_neeko_skin10_rebases_tags_and_sound_banks_if_installed() {
    let game = Path::new(r"D:\Riot Games\League of Legends\Game");
    if !game.join("DATA/FINAL/Champions/Neeko.wad.client").is_file() {
        return;
    }
    let mods_dir = std::env::temp_dir().join(format!("dekan_live_rebase_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&mods_dir); // ignore-ok: cleanup before live test
    let neeko = crate::generator::StandardChampion::open(game, "Neeko").expect("open neeko");
    let folder = neeko
        .build_mod(10, None, &mods_dir)
        .expect("build neeko 10");
    let skin0 = std::fs::read(
        mods_dir
            .join(&folder)
            .join("WAD/Neeko.wad.client/data/characters/neeko/skins/skin0.bin"),
    )
    .expect("read neeko skin0");
    let prop = parse_prop_file(&skin0).expect("parse neeko skin0");
    let entry = prop
        .entries
        .iter()
        .find(|e| e.class_hash == SKIN_DATA_CLASS)
        .expect("skin object");
    let fields = tree::parse_fields(&entry.body).expect("fields");
    let tags = units::string_items(
        tree::field(&fields, h("skinAudioProperties"))
            .and_then(Value::fields)
            .expect("audio props"),
        "tagEventList",
    );
    assert!(
        !tags
            .iter()
            .any(|t| t.to_ascii_lowercase().contains("skin10")),
        "rebased tagEventList must not leak skin10 tag: {tags:?}"
    );
    assert_eq!(
        tree::field(&fields, h("championSkinName"))
            .and_then(raw_text)
            .as_deref(),
        Some("Neeko")
    );
    let _ = std::fs::remove_dir_all(&mods_dir); // ignore-ok: cleanup after live test
}
