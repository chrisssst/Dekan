use super::*;

#[test]
fn test_standard_champion_live_wad_if_installed() {
    let game = Path::new(r"D:\Riot Games\League of Legends\Game");
    if !game
        .join("DATA")
        .join("FINAL")
        .join("Champions")
        .join("Zed.wad.client")
        .is_file()
    {
        return;
    }
    let champion = StandardChampion::open(game, "Zed").expect("open live zed wad");
    let zed_skins = champion.skin_numbers(50);
    assert!(zed_skins.contains(&1), "Zed must have skin 1");
    assert!(zed_skins.contains(&4), "Zed chroma 4 must have skin4.bin");
    let mods_dir = std::env::temp_dir().join(format!("dekan_live_mod_{}", std::process::id()));
    let folder = champion
        .build_mod(1, None, &mods_dir)
        .expect("build live mod");
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
    let _ = std::fs::remove_dir_all(&mods_dir); // ignore-ok: fixture cleanup

    let annie = StandardChampion::open(game, "Annie").expect("open live annie wad");
    assert!(annie.has_skin(1), "Annie must have skin 1 in live patch");
    let folder = annie
        .build_mod(1, None, &mods_dir)
        .expect("build live annie mod");
    assert_eq!(folder, "std_annie_1");
    let _ = std::fs::remove_dir_all(&mods_dir); // ignore-ok: fixture cleanup

    if let Ok(garen) = StandardChampion::open(game, "Garen") {
        if garen.has_skin(44) {
            let folder = garen
                .build_mod(44, None, &mods_dir)
                .expect("build live garen 44 mod");
            assert_eq!(folder, "std_garen_44");

            let skin_file = mods_dir
                .join(&folder)
                .join("WAD")
                .join("Garen.wad.client")
                .join("data")
                .join("characters")
                .join("garen")
                .join("skins")
                .join("skin0.bin");
            assert!(skin_file.is_file(), "skin0.bin must be generated");

            let skin_bytes = std::fs::read(&skin_file).expect("read generated skin0.bin");
            let skin_prop = parse_prop_file(&skin_bytes).expect("parse generated skin0.bin");
            assert_eq!(
                skin_prop.entries[0].key_hash,
                prop_key_hash("Characters/Garen/Skins/Skin0")
            );
            assert_eq!(
                skin_prop.links.first().map(String::as_str),
                Some("DATA/Characters/Garen/Skins/Skin44.bin"),
                "skin must link to source skin 44 bin first"
            );
            let source = garen
                .wad
                .read(wad_path_hash(&skin_bin("garen", 44)))
                .expect("read")
                .expect("skin44.bin");
            let source_links = parse_prop_file(&source).expect("parse skin44.bin").links;
            for link in &source_links {
                assert!(
                    skin_prop.links.contains(link),
                    "skin0.bin must keep skin44.bin's dependency {link}"
                );
            }

            assert!(
                skin_prop
                    .links
                    .contains(&"DATA/Characters/Garen/Animations/Skin44.bin".to_string()),
                "the skin's own animation graph is reached through its links"
            );
            assert!(
                !mods_dir
                    .join(&folder)
                    .join("WAD")
                    .join("Garen.wad.client")
                    .join("data")
                    .join("characters")
                    .join("garen")
                    .join("animations")
                    .join("skin0.bin")
                    .exists(),
                "the base animation graph is never replaced"
            );

            let _ = std::fs::remove_dir_all(&mods_dir); // ignore-ok: fixture cleanup
        }
    }

    if zed_skins.contains(&15) {
        let folder = champion
            .build_mod(15, None, &mods_dir)
            .expect("build live legendary mod");
        assert_eq!(folder, "std_zed_15");
        let characters = mods_dir
            .join(&folder)
            .join("WAD")
            .join("Zed.wad.client")
            .join("data")
            .join("characters")
            .join("zed");
        assert!(
            !characters.join("animations").join("skin0.bin").exists(),
            "the base animation graph is never replaced"
        );
        let skin = std::fs::read(characters.join("skins").join("skin0.bin")).expect("skin0");
        assert!(
            parse_prop_file(&skin)
                .expect("parse")
                .links
                .contains(&"DATA/Characters/Zed/Animations/Skin15.bin".to_string()),
            "the legendary graph is reached through the skin's links"
        );
        let _ = std::fs::remove_dir_all(&mods_dir); // ignore-ok: fixture cleanup
    }
}

#[test]
fn test_ultimate_skins_compatibility_if_installed() {
    let game = Path::new(r"D:\Riot Games\League of Legends\Game");
    if !game.join("DATA").join("FINAL").join("Champions").is_dir() {
        return;
    }

    let ultimates = [
        ("Lux", 7, "Elementalist Lux"),
        ("Sona", 6, "DJ Sona"),
        ("Udyr", 3, "Spirit Guard Udyr"),
        ("Ezreal", 5, "Pulsefire Ezreal"),
        ("MissFortune", 16, "Gun Goddess Miss Fortune"),
        ("Samira", 10, "Soul Fighter Samira"),
    ];

    let mods_dir = std::env::temp_dir().join(format!("dekan_ultimate_{}", std::process::id()));

    for (champ, skin, name) in ultimates {
        if let Ok(c) = StandardChampion::open(game, champ) {
            if c.has_skin(skin) {
                eprintln!("[ULTIMATE] {name} ({champ}, skin {skin})");

                let folder = c
                    .build_mod(skin, None, &mods_dir)
                    .expect("build ultimate mod");
                let skin_file = mods_dir
                    .join(&folder)
                    .join("WAD")
                    .join(format!("{champ}.wad.client"))
                    .join("data")
                    .join("characters")
                    .join(champ.to_ascii_lowercase())
                    .join("skins")
                    .join("skin0.bin");
                assert!(
                    skin_file.is_file(),
                    "ultimate skin0.bin must be generated for {name}"
                );

                let links = parse_prop_file(&std::fs::read(&skin_file).expect("skin0"))
                    .expect("parse skin0")
                    .links;
                let source = c
                    .read_skin_bin(&champ.to_ascii_lowercase(), skin)
                    .expect("read")
                    .expect("source bin");
                for link in parse_prop_file(&source).expect("parse source").links {
                    assert!(
                        links.contains(&link),
                        "{name} keeps the source dependency {link}, where its animation graph lives"
                    );
                }
                eprintln!("  -> Successfully built and verified {}!", folder);
                let _ = std::fs::remove_dir_all(&mods_dir); // ignore-ok: fixture cleanup
            } else {
                eprintln!(
                    "[ULTIMATE] {} ({}, skin {}) - not in wad",
                    name, champ, skin
                );
            }
        }
    }
}
