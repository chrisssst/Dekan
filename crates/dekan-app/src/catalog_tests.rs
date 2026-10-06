use super::*;
use dekan_core::library::{LibraryChroma, LibrarySkin};
use std::path::PathBuf;

fn library() -> ChampionLibrary {
    ChampionLibrary {
        champion_id: 238,
        skins: vec![
            LibrarySkin {
                id: 238001,
                package: PathBuf::from("238001.fantome"),
                chromas: vec![LibraryChroma {
                    id: 238004,
                    package: PathBuf::from("238004.fantome"),
                }],
            },
            LibrarySkin {
                id: 238999,
                package: PathBuf::from("238999.fantome"),
                chromas: vec![],
            },
        ],
    }
}

fn assets() -> ChampionAssets {
    serde_json::from_str(
            r##"{
                "id": 238, "name": "Zed", "alias": "Zed",
                "skins": [
                    { "id": 238000, "name": "Zed", "isBase": true },
                    { "id": 238001, "name": "Shockblade Zed",
                      "chromas": [ { "id": 238004, "name": "Shockblade Zed (Rose Quartz)", "colors": ["#E58BA5"] } ] },
                    { "id": 238002, "name": "SKT T1 Zed" }
                ]
            }"##,
        )
        .expect("fixture parses")
}

#[test]
fn test_only_what_is_on_disk_is_offered() {
    let catalog = build_catalog(&library(), Some(&assets()));
    let ids: Vec<u32> = catalog.skins.iter().map(|s| s.id).collect();

    assert_eq!(ids, vec![238001, 238999]);
    assert!(
        !ids.contains(&238002),
        "a skin the client knows but that is not installed must not be offered"
    );
    assert!(
        !ids.contains(&238000),
        "the base skin is never injected and never listed"
    );
}

#[test]
fn test_names_and_chroma_colors_come_from_the_client() {
    let catalog = build_catalog(&library(), Some(&assets()));
    let shockblade = &catalog.skins[0];

    assert_eq!(catalog.champion_name, "Zed");
    assert_eq!(shockblade.name, "Shockblade Zed");
    assert!(!shockblade.name_unknown);
    assert_eq!(shockblade.chromas[0].name, "Shockblade Zed (Rose Quartz)");
    assert_eq!(shockblade.chromas[0].color.as_deref(), Some("#E58BA5"));
}

#[test]
fn test_unknown_skin_is_listed_with_a_fallback_name_and_flagged() {
    let catalog = build_catalog(&library(), Some(&assets()));
    let unknown = &catalog.skins[1];

    assert_eq!(unknown.name, "Skin 238999");
    assert!(
        unknown.name_unknown,
        "the UI has to be able to tell a real name from a placeholder"
    );
}

#[test]
fn test_catalog_survives_a_silent_client() {
    let catalog = build_catalog(&library(), None);

    assert_eq!(catalog.champion_name, "#238");
    assert_eq!(
        catalog.skins.len(),
        2,
        "the library alone still yields a catalog"
    );
    assert!(catalog.skins.iter().all(|s| s.name_unknown));
    assert_eq!(catalog.entry_count(), 3);
}

#[test]
fn test_resolves_a_clicked_skin_and_a_clicked_chroma() {
    let catalog = build_catalog(&library(), Some(&assets()));

    let skin = catalog
        .resolve_target(238_001)
        .expect("a listed skin must resolve");
    assert_eq!(skin.champion_id, 238);
    assert_eq!(skin.skin_id, 238_001);
    assert_eq!(skin.chroma_id, None);
    assert_eq!(skin.package_entry_id(), 238_001);

    let chroma_id = catalog.skins[0].chromas[0].id;
    let chroma = catalog
        .resolve_target(chroma_id)
        .expect("a listed chroma must resolve");
    assert_eq!(
        chroma.skin_id, 238_001,
        "a chroma carries the skin it belongs to"
    );
    assert_eq!(chroma.chroma_id, Some(chroma_id));
    assert_eq!(chroma.package_entry_id(), chroma_id);
}

#[test]
fn test_refuses_an_id_that_is_not_in_the_catalog() {
    let catalog = build_catalog(&library(), Some(&assets()));

    assert!(
        catalog.resolve_target(238_000).is_none(),
        "the base skin is not an entry, so it cannot become a target"
    );
    assert!(
        catalog.resolve_target(1).is_none(),
        "an id from nowhere must not resolve to anything"
    );
}

#[test]
fn test_serializes_to_the_shape_the_ui_expects() {
    let catalog = build_catalog(&library(), Some(&assets()));
    let json = serde_json::to_value(&catalog).expect("serializes");

    assert_eq!(json["championName"], "Zed");
    assert_eq!(json["skins"][0]["chromas"][0]["color"], "#E58BA5");
    assert!(
        json["skins"][1]["chromas"]
            .as_array()
            .is_some_and(Vec::is_empty),
        "a skin with no chromas serializes an empty list, not null"
    );
    assert!(
        json["skins"][0].get("tile").is_none(),
        "an unfetched tile must be omitted, not serialized as null"
    );
}

#[test]
fn test_tile_data_uri_picks_the_mime_from_the_path_extension() {
    let png = tile_data_uri("/lol-game-data/assets/.../ZedSquare.png", b"\x89PNG");
    assert!(png.starts_with("data:image/png;base64,"));

    let jpg = tile_data_uri("/lol-game-data/assets/.../ZedSplash.jpg", b"\xff\xd8\xff");
    assert!(jpg.starts_with("data:image/jpeg;base64,"));
}

#[test]
fn test_empty_library_populates_from_client_assets() {
    let empty_lib = ChampionLibrary {
        champion_id: 238,
        skins: Vec::new(),
    };
    let catalog = build_catalog(&empty_lib, Some(&assets()));
    let ids: Vec<u32> = catalog.skins.iter().map(|s| s.id).collect();

    assert_eq!(ids, vec![238001, 238002]);
    assert_eq!(catalog.skins[0].name, "Shockblade Zed");
    assert_eq!(catalog.skins[1].name, "SKT T1 Zed");
    assert_eq!(catalog.champion_name, "Zed");
    assert_eq!(catalog.alias.as_deref(), Some("Zed"));
}

#[test]
fn test_client_forms_are_offered_under_their_skin() {
    let assets: ChampionAssets = serde_json::from_str(
        r##"{
            "id": 18, "name": "Tristana", "alias": "Tristana",
            "skins": [
                { "id": 18000, "name": "Tristana", "isBase": true },
                { "id": 18079, "name": "Risen Legend Tristana",
                  "chromas": [ { "id": 18081, "name": "Risen Legend Tristana (Ruby)" } ],
                  "questSkinInfo": { "tiers": [
                    { "id": 18079, "name": "Risen Legend Tristana" },
                    { "id": 18080, "name": "Immortalized Legend Tristana" }
                  ] } }
            ]
        }"##,
    )
    .expect("fixture parses");
    let empty = ChampionLibrary {
        champion_id: 18,
        skins: Vec::new(),
    };
    let from_client = build_catalog(&empty, Some(&assets));
    let target = from_client
        .resolve_target(18_080)
        .expect("the immortalized tier is selectable");
    assert_eq!(target.skin_id, 18_079);
    assert_eq!(target.chroma_id, Some(18_080));
    let form = from_client.skins[0]
        .chromas
        .iter()
        .find(|c| c.id == 18_080)
        .expect("listed");
    assert!(form.form);
    assert_eq!(form.name, "Immortalized Legend Tristana");
    assert!(
        !from_client.skins[0].chromas[0].form,
        "a chroma stays a chroma"
    );

    let installed = ChampionLibrary {
        champion_id: 18,
        skins: vec![LibrarySkin {
            id: 18_079,
            package: PathBuf::from("18079.fantome"),
            chromas: Vec::new(),
        }],
    };
    let from_library = build_catalog(&installed, Some(&assets));
    assert!(
        from_library.resolve_target(18_080).is_some(),
        "a form the library lacks is still offered, generated from the game"
    );
}

fn classic_annie() -> ChampionAssets {
    serde_json::from_str(
        r##"{
            "id": 60001, "name": "Annie", "alias": "Jade_Annie",
            "skins": [
                { "id": 60001000, "name": "Annie", "isBase": true },
                { "id": 60001001, "name": "Goth Annie" },
                { "id": 60001022, "name": "Cafe Cuties Annie",
                  "chromas": [ { "id": 60001023, "name": "Cafe Cuties Annie (Ruby)" },
                               { "id": 60001024, "name": "Cafe Cuties Annie (Pearl)" } ] },
                { "id": 60001301, "name": "Classic Annie",
                  "chromas": [ { "id": 60001302, "name": "Classic Annie (Goth)" },
                               { "id": 60001303, "name": "Classic Annie (Founder's Goth)" } ] }
            ]
        }"##,
    )
    .expect("the client's Classic shape parses")
}

#[test]
fn test_classic_offers_every_client_entry_the_game_has_a_file_for() {
    let numbers: std::collections::BTreeSet<u32> = [0, 1, 22, 23, 301, 302, 303].into();
    let (catalog, dropped) = build_classic_catalog(60001, Some(&classic_annie()), &numbers);
    assert!(catalog.classic);
    assert_eq!(catalog.champion_id, 60001);
    let offered: Vec<u32> = catalog
        .skins
        .iter()
        .flat_map(|s| std::iter::once(s.id).chain(s.chromas.iter().map(|c| c.id)))
        .collect();
    assert_eq!(
        offered,
        vec![60001001, 60001022, 60001023, 60001301, 60001302, 60001303],
        "Classic-only skins above 300 are offered; the base never is"
    );
    assert_eq!(
        dropped, 1,
        "the chroma without a Classic file (24) is left out"
    );
    let target = catalog.resolve_target(60001303).expect("selectable");
    assert_eq!(target.skin_id, 60001301);
    assert_eq!(
        dekan_classic::generator::skin_number(target.package_entry_id()),
        303
    );
}

#[test]
fn test_classic_without_client_data_lists_the_game_numbers() {
    let numbers: std::collections::BTreeSet<u32> = [0, 3, 301].into();
    let (catalog, _) = build_classic_catalog(60062, None, &numbers);
    let ids: Vec<u32> = catalog.skins.iter().map(|s| s.id).collect();
    assert_eq!(ids, vec![62003, 62301]);
    assert!(catalog.skins.iter().all(|s| s.name_unknown));
}

#[test]
fn test_the_chroma_preview_path_is_the_one_the_client_gave() {
    let assets: ChampionAssets = serde_json::from_str(
        r##"{
            "id": 238, "name": "Zed", "alias": "Zed",
            "skins": [
                { "id": 238000, "name": "Zed", "isBase": true },
                { "id": 238001, "name": "Shockblade Zed",
                  "chromas": [
                    { "id": 238004, "name": "Rose Quartz",
                      "chromaPath": "/lol-game-data/assets/v1/champion-chroma-images/238/238004.png" },
                    { "id": 238005, "name": "Catseye" }
                  ] }
            ]
        }"##,
    )
    .expect("fixture parses");
    let catalog = build_catalog(
        &ChampionLibrary {
            champion_id: 238,
            skins: Vec::new(),
        },
        Some(&assets),
    );
    assert_eq!(
        catalog.chroma_preview_path(238_004),
        Some("/lol-game-data/assets/v1/champion-chroma-images/238/238004.png")
    );
    assert_eq!(
        catalog.chroma_preview_path(238_005),
        None,
        "no path from the client, no preview"
    );
    assert_eq!(catalog.chroma_preview_path(999_999), None);
    assert_eq!(
        catalog.chroma_preview_paths(),
        vec![(
            238_004,
            "/lol-game-data/assets/v1/champion-chroma-images/238/238004.png".to_owned()
        )],
        "every chroma with a client path is fetched ahead, the rest are skipped"
    );
}
