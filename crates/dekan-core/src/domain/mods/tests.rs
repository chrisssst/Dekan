use super::*;

struct TempDir(PathBuf);
impl TempDir {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "dekan_mods_{tag}_{}_{}",
            std::process::id(),
            uuid_like()
        ));
        let _ = std::fs::remove_dir_all(&path); // ignore-ok: fixture may not exist yet
        std::fs::create_dir_all(&path).expect("temp dir");
        Self(path)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0); // ignore-ok: fixture cleanup
    }
}
fn uuid_like() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

fn make_mod(dir: &Path, content: &str) {
    std::fs::create_dir_all(dir.join("META")).expect("meta");
    std::fs::write(dir.join("META").join("info.json"), "{}").expect("info");
    std::fs::create_dir_all(dir.join(content)).expect("content");
    std::fs::write(dir.join(content).join("a.wad.client"), b"x").expect("wad");
}

#[test]
fn test_a_mod_folder_needs_meta_and_content_as_mkoverlay_does() {
    let tmp = TempDir::new("valid");
    let wad = tmp.0.join("wad_mod");
    make_mod(&wad, "WAD");
    let raw = tmp.0.join("raw_mod");
    make_mod(&raw, "RAW");
    let no_meta = tmp.0.join("no_meta");
    std::fs::create_dir_all(no_meta.join("WAD")).expect("wad");
    std::fs::write(no_meta.join("WAD").join("a.wad.client"), b"x").expect("file");
    let empty = tmp.0.join("empty_content");
    std::fs::create_dir_all(empty.join("META")).expect("meta");
    std::fs::write(empty.join("META").join("info.json"), "{}").expect("info");
    std::fs::create_dir_all(empty.join("WAD")).expect("wad");

    assert!(is_valid_mod_dir(&wad));
    assert!(
        is_valid_mod_dir(&raw),
        "RAW/ becomes a synthetic WAD in the overlay"
    );
    assert!(
        !is_valid_mod_dir(&no_meta),
        "the overlay builder requires META/info.json"
    );
    assert!(
        !is_valid_mod_dir(&empty),
        "a mod with nothing to merge is not a mod"
    );
}

#[test]
fn test_catalog_lists_categories_and_this_champions_skin_mods_in_every_layout() {
    let dekan = TempDir::new("dekan_root");
    make_mod(&dekan.0.join("maps").join("Winter Rift"), "WAD");
    make_mod(&dekan.0.join("skins").join("238").join("Neon Zed"), "WAD");
    make_mod(&dekan.0.join("skins").join("103").join("Ahri Mod"), "WAD");
    std::fs::create_dir_all(dekan.0.join("ui")).expect("ui");
    std::fs::write(dekan.0.join("ui").join("Clean HUD.fantome"), b"PK").expect("archive");

    make_mod(
        &dekan.0.join("skins").join("238000").join("Copied Zed"),
        "WAD",
    );
    make_mod(
        &dekan.0.join("skins").join("238005").join("Legacy Zed"),
        "WAD",
    );
    make_mod(&dekan.0.join("voiceover").join("JP Voices"), "RAW");

    let roots = [ModRoot {
        path: dekan.0.clone(),
        source: ModSource::Dekan,
    }];
    let catalog = scan_catalog(&roots, Some(238), &|_| false);

    let skin_ids: Vec<&str> = catalog.skin.iter().map(|e| e.id.as_str()).collect();
    assert!(skin_ids.contains(&"dekan:skins/238/Neon Zed"));
    assert!(skin_ids.contains(&"dekan:skins/238000/Copied Zed"));
    assert!(skin_ids.contains(&"dekan:skins/238005/Legacy Zed"));
    assert!(
        !skin_ids.iter().any(|id| id.contains("Ahri")),
        "another champion's skin mod must not be offered"
    );
    assert_eq!(catalog.map[0].id, "dekan:maps/Winter Rift");
    assert_eq!(catalog.others.len(), 2);
    assert_eq!(catalog.others[0].category, ModCategory::Ui);
    assert_eq!(catalog.others[0].package, ModPackage::Archive);
    assert_eq!(catalog.others[1].id, "dekan:voiceover/JP Voices");
}

#[test]
fn test_a_skin_mod_without_a_champion_folder_is_offered_when_it_belongs() {
    let root = TempDir::new("loose_skin");
    let skins = root.0.join("skins");
    std::fs::create_dir_all(&skins).expect("skins");

    std::fs::write(skins.join("big-smoke-nasus_1.0.1.fantome"), b"PK").expect("archive");
    make_mod(&skins.join("Loose Zed"), "WAD");
    make_mod(&skins.join("238").join("Neon Zed"), "WAD");
    let roots = [ModRoot {
        path: root.0.clone(),
        source: ModSource::Dekan,
    }];

    let nasus = scan_catalog(&roots, Some(75), &|e: &ModEntry| e.name.contains("nasus"));
    let ids: Vec<&str> = nasus.skin.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids, ["dekan:skins/big-smoke-nasus_1.0.1"]);
    assert_eq!(nasus.skin[0].package, ModPackage::Archive);

    let zed = scan_catalog(&roots, Some(238), &|e: &ModEntry| e.name.contains("Zed"));
    let ids: Vec<&str> = zed.skin.iter().map(|e| e.id.as_str()).collect();
    assert!(ids.contains(&"dekan:skins/Loose Zed"));
    assert!(ids.contains(&"dekan:skins/238/Neon Zed"));
    assert!(
        !ids.contains(&"dekan:skins/238"),
        "a champion folder is never listed as a mod"
    );

    let none = scan_catalog(&roots, None, &|_| true);
    assert!(none.skin.is_empty(), "no champion, no skin mods");
}

fn entry(id: &str, category: ModCategory) -> ModEntry {
    ModEntry {
        id: id.into(),
        name: id.into(),
        category,
        source: ModSource::Dekan,
        path: PathBuf::from(id),
        package: ModPackage::Directory,
        description: None,
    }
}

fn catalog() -> ModCatalog {
    ModCatalog {
        skin: vec![entry("dekan:skins/238/Neon", ModCategory::Skin)],
        map: vec![entry("dekan:maps/Winter", ModCategory::Map)],
        font: vec![],
        announcer: vec![],
        others: vec![
            entry("dekan:ui/HUD", ModCategory::Ui),
            entry("dekan:sfx/Pew", ModCategory::Sfx),
        ],
    }
}

#[test]
fn test_requests_are_validated_per_slot_and_never_guessed() {
    let request = ModSelectionView {
        skin: Some("dekan:skins/238/Neon".into()),
        map: Some("dekan:maps/Winter".into()),

        font: Some("dekan:maps/Winter".into()),
        announcer: Some("dekan:announcers/Ghost".into()),
        others: vec![
            "dekan:ui/HUD".into(),
            "dekan:ui/HUD".into(),
            "C:\\Windows\\System32".into(),
        ],
    };
    let (next, rejected) = catalog().apply_request(&ModSelection::default(), Some(238), &request);

    assert_eq!(
        next.skin.get(&238).map(String::as_str),
        Some("dekan:skins/238/Neon")
    );
    assert_eq!(next.map.as_deref(), Some("dekan:maps/Winter"));
    assert_eq!(next.font, None);
    assert_eq!(next.announcer, None);
    assert_eq!(
        next.others,
        vec!["dekan:ui/HUD".to_string()],
        "deduplicated"
    );
    let rejected_ids: Vec<&str> = rejected.iter().map(|r| r.id.as_str()).collect();
    assert!(rejected_ids.contains(&"dekan:maps/Winter"));
    assert!(rejected_ids.contains(&"dekan:announcers/Ghost"));
    assert!(rejected_ids.contains(&"C:\\Windows\\System32"));
}

#[test]
fn test_merge_order_and_fingerprint() {
    let (selection, _) = catalog().apply_request(
        &ModSelection::default(),
        Some(238),
        &ModSelectionView {
            skin: Some("dekan:skins/238/Neon".into()),
            map: Some("dekan:maps/Winter".into()),
            others: vec!["dekan:sfx/Pew".into(), "dekan:ui/HUD".into()],
            ..Default::default()
        },
    );
    assert_eq!(
        selection.ordered_ids(Some(238)),
        vec![
            "dekan:skins/238/Neon",
            "dekan:maps/Winter",
            "dekan:sfx/Pew",
            "dekan:ui/HUD"
        ]
    );

    assert_eq!(
        selection.ordered_ids(Some(103)),
        vec!["dekan:maps/Winter", "dekan:sfx/Pew", "dekan:ui/HUD"]
    );

    assert_eq!(ModSelection::default().fingerprint(Some(238)), 0);
    assert_ne!(selection.fingerprint(Some(238)), 0);
    assert_ne!(
        selection.fingerprint(Some(238)),
        selection.fingerprint(Some(103)),
        "a different set of mods must rebuild the overlay"
    );
    assert_eq!(
        selection.fingerprint(Some(238)),
        selection.clone().fingerprint(Some(238)),
        "stable across calls"
    );
}

#[test]
fn test_prune_drops_mods_that_left_the_disk() {
    let mut selection = ModSelection {
        skin: [
            (238, "dekan:skins/238/Gone".to_string()),
            (103, "dekan:skins/103/Kept".to_string()),
        ]
        .into_iter()
        .collect(),
        map: Some("dekan:maps/Winter".into()),
        font: Some("dekan:fonts/Gone".into()),
        announcer: None,
        others: vec!["dekan:ui/HUD".into(), "dekan:vfx/Gone".into()],
    };
    let dropped = selection.prune(&catalog(), Some(238));

    assert_eq!(dropped.len(), 3);
    assert_eq!(selection.map.as_deref(), Some("dekan:maps/Winter"));
    assert_eq!(selection.font, None);
    assert!(!selection.skin.contains_key(&238));
    assert!(
        selection.skin.contains_key(&103),
        "another champion was not scanned, so its choice is left alone"
    );
    assert_eq!(selection.others, vec!["dekan:ui/HUD".to_string()]);
}

#[test]
fn test_staged_names_are_overlay_safe_and_change_with_the_source() {
    let a = staged_name("dekan:maps/Spirit Blossom (2)", "100:1");
    let b = staged_name("dekan:maps/Spirit Blossom (2)", "100:2");
    assert!(a.starts_with(STAGED_PREFIX));
    assert!(a.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
    assert_ne!(a, b, "a replaced archive must not reuse the old extraction");
}
