use super::tests::*;
use super::*;

#[test]
fn test_a_companion_outside_the_registry_is_found_in_the_bins_and_retargeted() {
    let game = standard_game(
        "shadow",
        &[
            (
                wad_path_hash(&skin_bin("zed", 10)),
                companion_bin("Zed", 10),
            ),
            (
                wad_path_hash(&skin_bin("zedshadow", 10)),
                companion_bin("ZedShadow", 10),
            ),
            (
                wad_path_hash(&skin_bin("jade_zed", 10)),
                companion_bin("Jade_Zed", 10),
            ),
        ],
    );
    let mods_dir = game.join("mods");
    let champion = StandardChampion::open(&game, "Zed")
        .expect("open")
        .with_cache_dir(&game);
    assert!(champion.companions().contains("zedshadow"));
    assert!(
        !champion.companions().iter().any(|c| c.starts_with("jade_")),
        "Rift Classic characters belong to the Classic generator"
    );

    let folder = champion.build_mod(10, None, &mods_dir).expect("build");
    let shadow = generated_skin0(&mods_dir, &folder, "zedshadow").expect("shadow skin0.bin");
    assert_eq!(
        shadow.entries[0].key_hash,
        prop_key_hash("Characters/zedshadow/Skins/Skin0")
    );
    assert!(generated_skin0(&mods_dir, &folder, "jade_zed").is_none());
    assert!(game.join("companion_names_zed.json").is_file());
    let _ = std::fs::remove_dir_all(&game); // ignore-ok: fixture cleanup
}

#[test]
fn test_a_chroma_without_its_own_companion_bin_uses_the_base_skin_one() {
    let game = standard_game(
        "chroma",
        &[
            (
                wad_path_hash(&skin_bin("zed", 12)),
                companion_bin("Zed", 12),
            ),
            (
                wad_path_hash(&skin_bin("zedshadow", 10)),
                companion_bin("ZedShadow", 10),
            ),
        ],
    );
    let mods_dir = game.join("mods");
    let champion = StandardChampion::open(&game, "Zed").expect("open");

    let without_base = champion.build_mod(12, None, &mods_dir).expect("build");
    assert!(generated_skin0(&mods_dir, &without_base, "zedshadow").is_none());

    let with_base = champion.build_mod(12, Some(10), &mods_dir).expect("build");
    let shadow = generated_skin0(&mods_dir, &with_base, "zedshadow").expect("shadow skin0.bin");
    assert_eq!(
        shadow.links.first().map(String::as_str),
        Some("DATA/Characters/zedshadow/Skins/Skin10.bin")
    );
    let _ = std::fs::remove_dir_all(&game); // ignore-ok: fixture cleanup
}

#[test]
fn test_a_chroma_finds_its_parent_in_the_game_data_without_the_client() {
    let game = standard_game(
        "chroma_parent",
        &[
            (
                wad_path_hash(&skin_bin("zed", 12)),
                skin_object_bin("Zed", 12, 2, Some(10)),
            ),
            (
                wad_path_hash(&skin_bin("zedshadow", 10)),
                companion_bin("ZedShadow", 10),
            ),
        ],
    );
    let mods_dir = game.join("mods");
    let champion = StandardChampion::open(&game, "Zed").expect("open");
    assert_eq!(champion.parent_skin(12), Some(10));
    assert_eq!(champion.parent_skin(10), None, "no bin, no parent");

    let folder = champion.build_mod(12, None, &mods_dir).expect("build");
    let shadow = generated_skin0(&mods_dir, &folder, "zedshadow").expect("shadow skin0.bin");
    assert_eq!(
        shadow.links.first().map(String::as_str),
        Some("DATA/Characters/zedshadow/Skins/Skin10.bin"),
        "the companion comes from the parent skin the game names"
    );
    let _ = std::fs::remove_dir_all(&game); // ignore-ok: fixture cleanup
}

#[test]
fn test_the_client_names_the_classic_character_when_the_alias_does_not() {
    let game = std::env::temp_dir().join(format!("dekan_classic_wukong_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&game); // ignore-ok: fixture may not exist yet
    let champions = game.join("DATA").join("FINAL").join("Champions");
    std::fs::create_dir_all(&champions).expect("fixture dir");
    let wad = raw_wad(&[
        (
            wad_path_hash(&character_bin("jade_wukong")),
            b"PROP".to_vec(),
        ),
        (
            wad_path_hash(&skin_bin("jade_wukong", 3)),
            skin_bin_fixture("Jade_Wukong", 3),
        ),
    ]);
    std::fs::write(champions.join("MonkeyKing.wad.client"), wad).expect("write wad");
    let mods_dir = game.join("mods");
    let known = BTreeSet::new();

    let derived = ClassicChampion::open(&game, "MonkeyKing").expect("open");
    assert_eq!(derived.main_character(), "jade_monkeyking");
    assert!(
        derived
            .skin_numbers(derived.main_character(), 50)
            .is_empty()
    );
    assert!(
        derived.build_mod(3, &[0], &known, &mods_dir).is_err(),
        "the name derived from the archive does not exist in the game"
    );

    let named = ClassicChampion::open(&game, "MonkeyKing")
        .expect("open")
        .with_client_character(Some("Jade_Wukong"));
    assert_eq!(named.main_character(), "jade_wukong");
    assert_eq!(named.skin_numbers(named.main_character(), 50), vec![3]);
    let folder = named
        .build_mod(3, &[0], &known, &mods_dir)
        .expect("builds under the client's name");
    assert!(
        mods_dir
            .join(&folder)
            .join("WAD")
            .join("MonkeyKing.wad.client")
            .join("data")
            .join("characters")
            .join("jade_wukong")
            .join("skins")
            .join("skin0.bin")
            .is_file()
    );

    let unknown = ClassicChampion::open(&game, "MonkeyKing")
        .expect("open")
        .with_client_character(Some("Jade_Nobody"));
    assert_eq!(
        unknown.main_character(),
        "jade_monkeyking",
        "a client name absent from the archive is not trusted"
    );
    let _ = std::fs::remove_dir_all(&game); // ignore-ok: fixture cleanup
}

#[test]
fn test_the_companion_falls_back_only_to_a_real_other_base() {
    let game = standard_game(
        "fallback",
        &[
            (
                wad_path_hash(&skin_bin("zed", 12)),
                companion_bin("Zed", 12),
            ),
            (
                wad_path_hash(&skin_bin("zedshadow", 0)),
                companion_bin("ZedShadow", 0),
            ),
            (
                wad_path_hash(&skin_bin("zedshadow", 10)),
                companion_bin("ZedShadow", 10),
            ),
        ],
    );
    let champion = StandardChampion::open(&game, "Zed").expect("open");
    assert_eq!(
        champion.companion_source_skin("zedshadow", 12, Some(10)),
        Some(10)
    );
    assert_eq!(
        champion.companion_source_skin("zedshadow", 12, Some(0)),
        None,
        "the default skin is never a fallback"
    );
    assert_eq!(
        champion.companion_source_skin("zedshadow", 12, Some(12)),
        None
    );
    assert_eq!(
        champion.companion_source_skin("zedshadow", 10, None),
        Some(10)
    );
    assert_eq!(champion.companion_source_skin("zedshadow", 12, None), None);
    let _ = std::fs::remove_dir_all(&game); // ignore-ok: fixture cleanup
}
