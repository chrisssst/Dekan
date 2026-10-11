use super::tests::*;
use super::*;

#[test]
fn test_companions_are_recovered_from_bins_without_a_hash_table() {
    let game = std::env::temp_dir().join(format!("dekan_jade_bins_{}", std::process::id()));
    let champions = game.join("DATA").join("FINAL").join("Champions");
    std::fs::create_dir_all(&champions).expect("fixture dir");

    let mut bin = b"PROP".to_vec();
    bin.extend_from_slice(b"...Characters/Jade_Annie/Skins/Skin0...");
    bin.extend_from_slice(b"...Characters/Jade_AnnieTibbers/Skins/Skin0...");

    bin.extend_from_slice(b"...Characters/Jade_Soraka/Skins/Skin0...");

    let texture = b"DDS characters/jade_fake/x".to_vec();
    let wad = raw_wad(&[
        (1, bin),
        (2, texture),
        (
            wad_path_hash(&character_bin("jade_annie")),
            b"PROP".to_vec(),
        ),
        (
            wad_path_hash(&character_bin("jade_annietibbers")),
            b"PROP".to_vec(),
        ),
    ]);
    std::fs::write(champions.join("Annie.wad.client"), wad).expect("write wad");

    let champion = ClassicChampion::open(&game, "Annie").expect("open");
    let names = champion.jade_names_in_bins();
    assert!(names.contains("jade_annietibbers"));
    assert!(!names.contains("jade_fake"), "{names:?}");
    assert_eq!(
        champion.present_characters(&names),
        vec!["jade_annie".to_owned(), "jade_annietibbers".to_owned()]
    );

    let cached = champion.jade_names_from_bins_cached(&game);
    assert_eq!(cached, names);
    assert!(game.join("classic_bin_names_annie.json").is_file());
    assert_eq!(champion.jade_names_from_bins_cached(&game), names);

    let _ = std::fs::remove_dir_all(&game); // ignore-ok: fixture cleanup
}

#[test]
fn test_standard_champion_generates_slot_0_redirection() {
    let game = std::env::temp_dir().join(format!("dekan_std_wad_{}", std::process::id()));
    let champions = game.join("DATA").join("FINAL").join("Champions");
    std::fs::create_dir_all(&champions).expect("fixture dir");

    let skin_bin_content = skin_bin_fixture("Zed", 1);
    let wad = raw_wad(&[(wad_path_hash(&skin_bin("zed", 1)), skin_bin_content)]);
    std::fs::write(champions.join("Zed.wad.client"), wad).expect("write wad");

    let mods_dir = game.join("mods");
    std::fs::create_dir_all(&mods_dir).expect("mods dir");

    let champion = StandardChampion::open(&game, "Zed").expect("open");
    assert!(champion.has_skin(1));
    assert!(!champion.has_skin(2));

    let folder = champion.build_mod(1, None, &mods_dir).expect("build mod");
    assert_eq!(folder, "std_zed_1");
    assert!(
        mods_dir
            .join(&folder)
            .join("WAD")
            .join("Zed.wad.client")
            .join("data")
            .join("characters")
            .join("zed")
            .join("skins")
            .join("skin0.bin")
            .is_file()
    );
    assert!(
        mods_dir
            .join(&folder)
            .join("META")
            .join("info.json")
            .is_file()
    );

    let _ = std::fs::remove_dir_all(&game); // ignore-ok: fixture cleanup
}

#[test]
fn test_standard_champion_retargets_the_companion_too() {
    let game = std::env::temp_dir().join(format!("dekan_std_pet_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&game); // ignore-ok: fixture may not exist yet
    let champions = game.join("DATA").join("FINAL").join("Champions");
    std::fs::create_dir_all(&champions).expect("fixture dir");
    let wad = raw_wad(&[
        (
            wad_path_hash(&skin_bin("annie", 5)),
            skin_bin_fixture("Annie", 5),
        ),
        (wad_path_hash(&skin_bin("annietibbers", 5)), tibbers_bin(5)),
    ]);
    std::fs::write(champions.join("Annie.wad.client"), wad).expect("write wad");
    let mods_dir = game.join("mods");

    let folder = StandardChampion::open(&game, "Annie")
        .expect("open")
        .build_mod(5, None, &mods_dir)
        .expect("build");
    let chars = mods_dir
        .join(&folder)
        .join("WAD")
        .join("Annie.wad.client")
        .join("data")
        .join("characters");
    for (dir, character) in [("annie", "Annie"), ("annietibbers", "annietibbers")] {
        let bytes = std::fs::read(chars.join(dir).join("skins").join("skin0.bin"))
            .unwrap_or_else(|e| panic!("{dir} skin0.bin: {e}"));
        let prop = parse_prop_file(&bytes).expect("valid PROP");
        let keys: Vec<u32> = prop.entries.iter().map(|e| e.key_hash).collect();
        assert!(
            keys.contains(&prop_key_hash(&format!(
                "Characters/{character}/Skins/Skin0"
            ))),
            "{dir}: the skin object is re-keyed to slot 0"
        );
        assert!(
            !keys.contains(&prop_key_hash(&format!(
                "Characters/{character}/Skins/Skin5"
            ))),
            "{dir}: nothing left under the source slot"
        );
        assert_eq!(
            prop.links.first(),
            Some(&format!("DATA/Characters/{character}/Skins/Skin5.bin")),
            "{dir}: links the original skin bin for everything else"
        );
        assert!(
            prop.links
                .contains(&"DATA/Characters/Annie/Annie.bin".to_string()),
            "{dir}: keeps the source bin's own dependencies"
        );
    }
    let _ = std::fs::remove_dir_all(&game); // ignore-ok: fixture cleanup
}

#[test]
fn test_a_broken_companion_bin_still_yields_the_champion_skin() {
    let game = std::env::temp_dir().join(format!("dekan_std_badpet_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&game); // ignore-ok: fixture may not exist yet
    let champions = game.join("DATA").join("FINAL").join("Champions");
    std::fs::create_dir_all(&champions).expect("fixture dir");
    let wad = raw_wad(&[
        (
            wad_path_hash(&skin_bin("annie", 5)),
            skin_bin_fixture("Annie", 5),
        ),
        (
            wad_path_hash(&skin_bin("annietibbers", 5)),
            b"not a prop file".to_vec(),
        ),
    ]);
    std::fs::write(champions.join("Annie.wad.client"), wad).expect("write wad");
    let mods_dir = game.join("mods");
    let folder = StandardChampion::open(&game, "Annie")
        .expect("open")
        .build_mod(5, None, &mods_dir)
        .expect("the champion skin is still built");
    let chars = mods_dir
        .join(&folder)
        .join("WAD")
        .join("Annie.wad.client")
        .join("data")
        .join("characters");
    assert!(
        chars
            .join("annie")
            .join("skins")
            .join("skin0.bin")
            .is_file()
    );
    assert!(
        !chars.join("annietibbers").exists(),
        "no half-written companion"
    );
    let _ = std::fs::remove_dir_all(&game); // ignore-ok: fixture cleanup
}
