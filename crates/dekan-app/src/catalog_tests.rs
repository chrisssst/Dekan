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
                "id": 238, "name": "Zed",
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
