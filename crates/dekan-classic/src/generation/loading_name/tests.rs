use dekan_wad::hash::wad_path_hash;

use super::*;

fn make_rst_v5(texts: &[(&str, &str)]) -> Vec<u8> {
    let mut items: Vec<(u64, &str)> = texts.iter().map(|(k, v)| (rst::key_hash(k), *v)).collect();
    items.sort_by_key(|(h, _)| *h);
    let mut entries = Vec::with_capacity(items.len());
    let mut body = Vec::new();
    for (hash, text) in items {
        entries.push(hash | ((body.len() as u64) << 38));
        body.extend_from_slice(text.as_bytes());
        body.push(0);
    }
    let mut out = Vec::new();
    out.extend_from_slice(b"RST\x05");
    out.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    for e in entries {
        out.extend_from_slice(&e.to_le_bytes());
    }
    out.extend_from_slice(&body);
    out
}

fn make_wad_v3(entries: &[(u64, &[u8])]) -> Vec<u8> {
    let header_len = 272usize;
    let toc_len = entries.len() * 32;
    let mut offset = (header_len + toc_len) as u32;
    let mut out = Vec::new();
    out.extend_from_slice(b"RW\x03\x03");
    out.resize(260, 0);
    out.extend_from_slice(&0u64.to_le_bytes());
    out.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    for (hash, data) in entries {
        let len = data.len() as u32;
        out.extend_from_slice(&hash.to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&len.to_le_bytes());
        out.push(0);
        out.push(0);
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u64.to_le_bytes());
        offset += len;
    }
    for (_, data) in entries {
        out.extend_from_slice(data);
    }
    out
}

fn make_record(key: &str) -> Vec<u8> {
    let mut out = vec![0x01, 0x02];
    out.extend_from_slice(&(key.len() as u16).to_le_bytes());
    out.extend_from_slice(key.as_bytes());
    out.push(0x03);
    out
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "dekan_loading_name_test_{name}_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn setup_game(
    root: &Path,
    languages: &[(&str, &str)],
    locale: Option<&str>,
) -> (PathBuf, PathBuf, PathBuf) {
    let game = root.join("League of Legends").join("Game");
    let loc_dir = game.join("DATA").join("FINAL").join("Localized");
    std::fs::create_dir_all(&loc_dir).expect("mkdir localized");
    for (lang, skin_name) in languages {
        let table = make_rst_v5(&[
            ("game_character_displayname_Zed", "Zed"),
            ("game_character_displayname_Yasuo", "Yasuo"),
            ("game_character_skin_displayname_Zed_1", skin_name),
        ]);
        let wad = make_wad_v3(&[(rst::table_hash(), &table)]);
        std::fs::write(loc_dir.join(format!("Global.{lang}.wad.client")), wad).expect("write wad");
    }
    let champ_dir = game.join("DATA").join("FINAL").join("Champions");
    std::fs::create_dir_all(&champ_dir).expect("mkdir champs");
    let zed_bin = make_record("game_character_displayname_Zed");
    let shadow_bin = make_record("game_character_displayname_Zed");
    let zed_wad = make_wad_v3(&[
        (wad_path_hash("data/characters/zed/zed.bin"), &zed_bin),
        (
            wad_path_hash("data/characters/zedshadow/zedshadow.bin"),
            &shadow_bin,
        ),
    ]);
    std::fs::write(champ_dir.join("Zed.wad.client"), zed_wad).expect("write zed wad");
    if let Some(loc) = locale {
        let cfg = root.join("League of Legends").join("Config");
        std::fs::create_dir_all(&cfg).expect("mkdir config");
        std::fs::write(
            cfg.join("LeagueClientSettings.yaml"),
            format!("install:\n    globals:\n        locale: \"{loc}\"\n        region: \"BR\"\n"),
        )
        .expect("write settings");
    }
    let mods = root.join("mods");
    let skin_mod = mods.join("std_zed_1");
    std::fs::create_dir_all(skin_mod.join("WAD").join("Zed.wad.client")).expect("mkdir skin mod");
    (game, mods, skin_mod)
}

#[test]
fn rst_v5_round_trip_preserves_utf8_and_other_entries() {
    let table = make_rst_v5(&[
        ("game_character_displayname_Zed", "Zed"),
        ("game_character_displayname_Yasuo", "Yasuo"),
    ]);
    let updated = rst::with_texts(
        &table,
        &[
            (
                "game_character_displayname_Zed".into(),
                "Lâmina do Trovão".into(),
            ),
            ("bllt_character_displayname_Zed".into(), "Zed".into()),
        ],
    )
    .expect("patch rst");

    assert_eq!(
        rst::read_text(&updated, rst::key_hash("game_character_displayname_Zed")).as_deref(),
        Some("Lâmina do Trovão")
    );
    assert_eq!(
        rst::read_text(&updated, rst::key_hash("bllt_character_displayname_Zed")).as_deref(),
        Some("Zed")
    );
    assert_eq!(
        rst::read_text(&updated, rst::key_hash("game_character_displayname_Yasuo")).as_deref(),
        Some("Yasuo")
    );
}

#[test]
fn record_key_patch_preserves_length_and_is_case_insensitive() {
    let raw = make_record("Game_Character_DisplayName_Zed");
    let patched = rst::patch_record_key(&raw, "zed").expect("patch record");
    assert_eq!(patched.len(), raw.len());
    assert!(
        patched
            .windows("bllt_character_displayname_Zed".len())
            .any(|w| w == b"bllt_character_displayname_Zed")
    );
    assert!(rst::patch_record_key(b"unrelated", "Zed").is_none());
}

#[test]
fn en_us_install_uses_string_overrides_and_repoints_champion_and_forms() {
    let root = temp_dir("en_us");
    let (game, mods, skin_mod) = setup_game(&root, &[("en_US", "Shockblade Zed")], Some("en_US"));

    let built = build_loading_name_mod(
        &game,
        &mods,
        &[LoadingNamePick {
            mod_folder: skin_mod,
            skin_id: 238_001,
            parent_skin_id: None,
        }],
        &[],
    );
    assert_eq!(built.as_deref(), Some(MOD_FOLDER));

    let out = mods.join(MOD_FOLDER);
    assert!(!out.join("RAW/DATA/Menu/en_US/lol.stringtable").exists());
    let info: serde_json::Value =
        serde_json::from_slice(&std::fs::read(out.join("META/info.json")).expect("read info.json"))
            .expect("parse info.json");
    assert_eq!(
        info["Layers"]["base"]["StringOverrides"]["en_us"]["game_character_displayname_Zed"],
        "Shockblade Zed"
    );
    assert_eq!(
        info["Layers"]["base"]["StringOverrides"]["en_us"]["bllt_character_displayname_Zed"],
        "Zed"
    );
    let zed_bin = std::fs::read(out.join("RAW/data/characters/zed/zed.bin")).expect("read zed.bin");
    let shadow_bin = std::fs::read(out.join("RAW/data/characters/zedshadow/zedshadow.bin"))
        .expect("read zedshadow.bin");
    assert!(zed_bin.windows(4).any(|w| w == b"bllt"));
    assert!(shadow_bin.windows(4).any(|w| w == b"bllt"));
    let _ = std::fs::remove_dir_all(&root); // ignore-ok: best-effort test temp cleanup
}

#[test]
fn non_en_us_install_writes_loose_table_and_skips_when_other_mod_ships_table() {
    let root = temp_dir("pt_br");
    let (game, mods, skin_mod) =
        setup_game(&root, &[("pt_BR", "Zed Lâmina do Trovão")], Some("pt_BR"));

    let font = mods.join("custom_font");
    std::fs::create_dir_all(font.join("RAW/DATA/Menu/en_US")).expect("mkdir font");
    std::fs::write(font.join("RAW/DATA/Menu/en_US/lol.stringtable"), b"RST\x05")
        .expect("write font table");

    assert!(
        build_loading_name_mod(
            &game,
            &mods,
            &[LoadingNamePick {
                mod_folder: skin_mod.clone(),
                skin_id: 238_001,
                parent_skin_id: None,
            }],
            std::slice::from_ref(&font),
        )
        .is_none()
    );

    let built = build_loading_name_mod(
        &game,
        &mods,
        &[LoadingNamePick {
            mod_folder: skin_mod,
            skin_id: 238_001,
            parent_skin_id: None,
        }],
        &[],
    );
    assert_eq!(built.as_deref(), Some(MOD_FOLDER));
    let table = std::fs::read(
        mods.join(MOD_FOLDER)
            .join("RAW/DATA/Menu/en_US/lol.stringtable"),
    )
    .expect("read loose table");
    assert_eq!(
        rst::read_text(&table, rst::key_hash("game_character_displayname_Zed")).as_deref(),
        Some("Zed Lâmina do Trovão")
    );
    assert_eq!(
        rst::read_text(&table, rst::key_hash("bllt_character_displayname_Zed")).as_deref(),
        Some("Zed")
    );
    let _ = std::fs::remove_dir_all(&root); // ignore-ok: best-effort test temp cleanup
}

#[test]
fn chroma_inherits_parent_skin_name_and_base_skin_builds_nothing() {
    let root = temp_dir("chroma");
    let (game, mods, skin_mod) = setup_game(&root, &[("en_US", "Shockblade Zed")], Some("en_US"));

    assert!(
        build_loading_name_mod(
            &game,
            &mods,
            &[LoadingNamePick {
                mod_folder: skin_mod.clone(),
                skin_id: 238_000,
                parent_skin_id: None,
            }],
            &[],
        )
        .is_none()
    );

    let built = build_loading_name_mod(
        &game,
        &mods,
        &[LoadingNamePick {
            mod_folder: skin_mod,
            skin_id: 238_004,
            parent_skin_id: Some(238_001),
        }],
        &[],
    );
    assert_eq!(built.as_deref(), Some(MOD_FOLDER));
    let info: serde_json::Value = serde_json::from_slice(
        &std::fs::read(mods.join(MOD_FOLDER).join("META/info.json")).expect("read info"),
    )
    .expect("parse info");
    assert_eq!(info["Description"], "Shockblade Zed");
    let _ = std::fs::remove_dir_all(&root); // ignore-ok: best-effort test temp cleanup
}

#[test]
fn non_en_us_locale_alongside_en_us_wad_is_unsupported() {
    let root = temp_dir("mixed_locale");
    let (game, mods, skin_mod) = setup_game(
        &root,
        &[
            ("en_US", "Shockblade Zed"),
            ("pt_BR", "Zed Lâmina do Trovão"),
        ],
        Some("pt_BR"),
    );
    assert_eq!(strategy_for(&game), Strategy::Unsupported);
    assert!(
        build_loading_name_mod(
            &game,
            &mods,
            &[LoadingNamePick {
                mod_folder: skin_mod,
                skin_id: 238_001,
                parent_skin_id: None,
            }],
            &[],
        )
        .is_none()
    );
    let _ = std::fs::remove_dir_all(&root); // ignore-ok: best-effort test temp cleanup
}
