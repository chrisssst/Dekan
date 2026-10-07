use super::*;
use dekan_core::mods::ModSelectionView;

struct TempDir(PathBuf);
impl TempDir {
    fn new(tag: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("dekan_mods_store_{tag}_{}", std::process::id()));
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

fn make_mod(dir: &Path) {
    std::fs::create_dir_all(dir.join("META")).expect("meta");
    std::fs::write(dir.join("META").join("info.json"), "{}").expect("info");
    std::fs::create_dir_all(dir.join("WAD")).expect("wad");
    std::fs::write(dir.join("WAD").join("Map11.wad.client"), b"wad").expect("file");
}

fn make_archive(path: &Path) {
    make_archive_with(path, "WAD/UI.wad.client");
}

fn make_archive_with(path: &Path, content: &str) {
    let file = std::fs::File::create(path).expect("archive");
    let mut writer = zip::ZipWriter::new(file);
    let options: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
    writer.start_file("META/info.json", options).expect("meta");
    std::io::Write::write_all(&mut writer, b"{}").expect("meta body");
    writer.start_file(content, options).expect("content");
    std::io::Write::write_all(&mut writer, b"x").expect("content body");
    writer.finish().expect("finish");
}

#[test]
fn test_selection_round_trips_and_a_damaged_file_is_set_aside() {
    let tmp = TempDir::new("persist");
    let selection = ModSelection {
        map: Some("dekan:maps/Winter".into()),
        others: vec!["dekan:ui/HUD".into()],
        skin: [(238, "dekan:skins/238/Neon".into())]
            .into_iter()
            .collect(),
        ..Default::default()
    };
    save_selection(&tmp.0, &selection);
    assert_eq!(load_selection(&tmp.0), selection);

    std::fs::write(tmp.0.join(SELECTION_FILE), b"{not json").expect("damage");
    assert_eq!(load_selection(&tmp.0), ModSelection::default());
    assert!(
        tmp.0.join("mods_selection.json.unreadable").is_file(),
        "the damaged file is kept for recovery, not overwritten"
    );
}

#[test]
fn test_selected_mods_are_staged_in_merge_order_and_stale_ones_removed() {
    let root = TempDir::new("root");
    let staging = TempDir::new("staging");
    make_mod(&root.0.join("maps").join("Winter Rift"));
    std::fs::create_dir_all(root.0.join("ui")).expect("ui");
    make_archive(&root.0.join("ui").join("Clean HUD.fantome"));
    std::fs::create_dir_all(staging.0.join("cm_deadbeefdeadbeef")).expect("stale");
    std::fs::create_dir_all(staging.0.join("238_238001")).expect("library skin");

    let roots = [ModRoot {
        path: root.0.clone(),
        source: ModSource::Dekan,
    }];
    let catalog = scan_catalog(&roots, Some(238), &|_| false);
    let (selection, rejected) = catalog.apply_request(
        &ModSelection::default(),
        Some(238),
        &ModSelectionView {
            map: Some("dekan:maps/Winter Rift".into()),
            others: vec!["dekan:ui/Clean HUD".into()],
            ..Default::default()
        },
    );
    assert!(rejected.is_empty(), "{rejected:?}");

    let staged = stage_selected(&catalog, &selection, Some(238), &staging.0);
    assert_eq!(staged.len(), 2);
    for name in &staged {
        assert!(
            is_valid_mod_dir(&staging.0.join(name)),
            "{name} must be a usable mod"
        );
    }
    assert!(
        !staging.0.join("cm_deadbeefdeadbeef").exists(),
        "an unselected staged mod is cleaned up"
    );
    assert!(
        staging.0.join("238_238001").exists(),
        "library skins are never touched by the custom mod cleanup"
    );
    assert!(
        root.0
            .join("maps")
            .join("Winter Rift")
            .join("WAD")
            .join("Map11.wad.client")
            .is_file()
    );

    let again = stage_selected(&catalog, &selection, Some(238), &staging.0);
    assert_eq!(again, staged);
}

#[test]
fn test_a_vanished_mod_is_skipped_not_fatal() {
    let root = TempDir::new("vanish_root");
    let staging = TempDir::new("vanish_staging");
    make_mod(&root.0.join("maps").join("Winter Rift"));
    let roots = [ModRoot {
        path: root.0.clone(),
        source: ModSource::Dekan,
    }];
    let catalog = scan_catalog(&roots, None, &|_| false);
    let selection = ModSelection {
        map: Some("dekan:maps/Winter Rift".into()),
        font: Some("dekan:fonts/Deleted".into()),
        ..Default::default()
    };
    let staged = stage_selected(&catalog, &selection, None, &staging.0);
    assert_eq!(
        staged.len(),
        1,
        "the missing font is skipped, the map still loads"
    );
}

#[test]
fn test_import_archive_valid_and_name_collision() {
    let root = TempDir::new("import_root");
    let source_dir = TempDir::new("import_source");
    let source_file = source_dir.0.join("Neon Strike.fantome");
    make_archive(&source_file);

    let imported = import_archive(&root.0, ModCategory::Map, None, &source_file).expect("import");
    assert_eq!(imported, root.0.join("maps").join("Neon Strike.fantome"));
    assert!(imported.is_file());

    let second =
        import_archive(&root.0, ModCategory::Map, None, &source_file).expect("second import");
    assert_eq!(second, root.0.join("maps").join("Neon Strike (2).fantome"));
    assert!(second.is_file());
}

#[test]
fn test_import_archive_wrong_extension() {
    let root = TempDir::new("import_ext_root");
    let source_dir = TempDir::new("import_ext_source");
    let source_file = source_dir.0.join("mod.txt");
    std::fs::write(&source_file, b"not a mod").expect("write");

    let err = import_archive(&root.0, ModCategory::Map, None, &source_file).unwrap_err();
    assert_eq!(err, ImportRefusal::UnsupportedExtension);
    for language in [
        dekan_platform::i18n::Language::Turkish,
        dekan_platform::i18n::Language::English,
    ] {
        assert!(err.describe(language.text()).contains(".fantome"));
    }
}

#[test]
fn test_import_archive_no_content() {
    let root = TempDir::new("import_nocontent_root");
    let source_dir = TempDir::new("import_nocontent_source");
    let source_file = source_dir.0.join("Empty.fantome");

    let file = std::fs::File::create(&source_file).expect("file");
    let mut writer = zip::ZipWriter::new(file);
    let options: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
    writer.start_file("META/info.json", options).expect("meta");
    std::io::Write::write_all(&mut writer, b"{}").expect("meta body");
    writer.finish().expect("finish");

    let err = import_archive(&root.0, ModCategory::Map, None, &source_file).unwrap_err();
    assert_eq!(err, ImportRefusal::NoContent);
}

#[test]
fn test_a_skin_mod_that_names_no_champion_takes_the_one_in_champ_select() {
    let root = TempDir::new("import_skin_root");
    let source_dir = TempDir::new("import_skin_source");
    let source_file = source_dir.0.join("ZedMod.fantome");
    make_archive_with(&source_file, "RAW/assets/zed.dds");

    let err = import_archive(&root.0, ModCategory::Skin, None, &source_file).unwrap_err();
    assert_eq!(err, ImportRefusal::NoChampion);

    let imported =
        import_archive(&root.0, ModCategory::Skin, Some(238), &source_file).expect("skin import");
    assert_eq!(
        imported,
        root.0.join("skins").join("238").join("ZedMod.fantome")
    );
    assert!(imported.is_file());
}

#[test]
fn test_a_skin_mod_is_matched_to_its_champion_by_content_not_by_folder() {
    let root = TempDir::new("loose_root");
    let source_dir = TempDir::new("loose_source");
    let source_file = source_dir.0.join("big-smoke-nasus_1.0.1.fantome");

    make_archive_with(&source_file, "WAD/Nasus.wad.client");

    let imported =
        import_archive(&root.0, ModCategory::Skin, Some(238), &source_file).expect("import");
    assert_eq!(
        imported,
        root.0.join("skins").join("big-smoke-nasus_1.0.1.fantome")
    );

    let roots = [ModRoot {
        path: root.0.clone(),
        source: ModSource::Dekan,
    }];
    let for_alias =
        |alias: &'static str| move |entry: &ModEntry| belongs_to_alias(entry, Some(alias));
    let nasus = scan_catalog(&roots, Some(75), &for_alias("Nasus"));
    assert_eq!(nasus.skin.len(), 1, "offered to Nasus");
    let zed = scan_catalog(&roots, Some(238), &for_alias("Zed"));
    assert!(zed.skin.is_empty(), "never offered to another champion");
    let unknown = scan_catalog(&roots, Some(75), &|e: &ModEntry| belongs_to_alias(e, None));
    assert!(unknown.skin.is_empty(), "no alias, no guess");

    let folder = root.0.join("skins").join("Neon Zed");
    std::fs::create_dir_all(folder.join("META")).expect("meta");
    std::fs::write(folder.join("META").join("info.json"), "{}").expect("info");
    let wad = folder
        .join("WAD")
        .join("Champions")
        .join("Zed.pt_BR.wad.client");
    std::fs::create_dir_all(&wad).expect("wad folder");
    std::fs::write(wad.join("skin.bin"), b"x").expect("file");
    let zed = scan_catalog(&roots, Some(238), &for_alias("Zed"));
    assert_eq!(zed.skin.len(), 1);
    assert_eq!(zed.skin[0].id, "dekan:skins/Neon Zed");
}

fn modpkg_with_one_raw_chunk(wad: &str, path_hash: u64, content: &[u8]) -> Vec<u8> {
    let mut out = dekan_wad::modpkg::MAGIC.to_vec();
    for value in [1u32, 0, 1, 1] {
        out.extend_from_slice(&value.to_le_bytes());
    }
    out.extend_from_slice(&4u32.to_le_bytes());
    out.extend_from_slice(b"base");
    out.extend_from_slice(&0i32.to_le_bytes());
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(b"skin.bin\0");
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(wad.as_bytes());
    out.push(0);
    out.resize(out.len().next_multiple_of(8), 0);
    let data_offset = (out.len() + 61) as u64;
    let checksum = dekan_wad::hash::content_checksum(content);
    out.extend_from_slice(&path_hash.to_le_bytes());
    out.extend_from_slice(&data_offset.to_le_bytes());
    out.push(0);
    out.extend_from_slice(&(content.len() as u64).to_le_bytes());
    out.extend_from_slice(&(content.len() as u64).to_le_bytes());
    out.extend_from_slice(&checksum.to_le_bytes());
    out.extend_from_slice(&checksum.to_le_bytes());
    for value in [0u32, 0, 0] {
        out.extend_from_slice(&value.to_le_bytes());
    }
    out.extend_from_slice(content);
    out
}

#[test]
fn test_a_modpkg_is_imported_offered_to_its_champion_and_staged_as_a_mod_folder() {
    let root = TempDir::new("modpkg_root");
    let source = TempDir::new("modpkg_source");
    let staging = TempDir::new("modpkg_staging");
    let file = source.0.join("Sniper Varus.modpkg");
    std::fs::write(
        &file,
        modpkg_with_one_raw_chunk("varus.wad.client", 0xABCD, b"skin bin"),
    )
    .expect("write");

    let imported = import_archive(&root.0, ModCategory::Skin, None, &file).expect("import");
    assert_eq!(imported, root.0.join("skins").join("Sniper Varus.modpkg"));

    let roots = [ModRoot {
        path: root.0.clone(),
        source: ModSource::Dekan,
    }];
    let catalog = scan_catalog(&roots, Some(110), &|entry: &ModEntry| {
        belongs_to_alias(entry, Some("Varus"))
    });
    assert_eq!(catalog.skin.len(), 1, "offered to Varus");

    let (selection, rejected) = catalog.apply_request(
        &ModSelection::default(),
        Some(110),
        &ModSelectionView {
            skin: Some(catalog.skin[0].id.clone()),
            ..Default::default()
        },
    );
    assert!(rejected.is_empty(), "{rejected:?}");
    let staged = stage_selected(&catalog, &selection, Some(110), &staging.0);
    assert_eq!(staged.len(), 1);
    let dir = staging.0.join(&staged[0]);
    assert!(is_valid_mod_dir(&dir));
    let wad = dekan_wad::WadFile::open(&dir.join("WAD").join("varus.wad.client")).expect("wad");
    assert_eq!(
        wad.read(0xABCD).expect("read").as_deref(),
        Some(&b"skin bin"[..])
    );

    let broken = source.0.join("broken.modpkg");
    std::fs::write(&broken, b"_modpkg_ truncated").expect("write");
    assert!(matches!(
        import_archive(&root.0, ModCategory::Skin, None, &broken),
        Err(ImportRefusal::NotAModPackage(_))
    ));
}

#[test]
fn the_space_an_archive_needs_is_what_it_unpacks_to_not_its_file_size() {
    use std::io::Write;
    let dir = TempDir::new("unpacked_size");
    let zipped = dir.0.join("bomb.fantome");
    {
        let mut writer = zip::ZipWriter::new(std::fs::File::create(&zipped).expect("zip"));
        let deflated = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        writer
            .start_file("WAD/Zed.wad.client", deflated)
            .expect("entry");
        writer
            .write_all(&vec![0u8; 64 * 1024 * 1024])
            .expect("zeros");
        writer.finish().expect("finish");
    }
    let on_disk = std::fs::metadata(&zipped).expect("meta").len();
    assert!(on_disk < 1024 * 1024, "{on_disk}");
    assert_eq!(unpacked_size(&zipped), 64 * 1024 * 1024);

    let package = dir.0.join("skin.modpkg");
    std::fs::write(
        &package,
        modpkg_with_one_raw_chunk("zed.wad.client", 1, &[7; 1000]),
    )
    .expect("pkg");
    assert_eq!(
        unpacked_size(&package),
        std::fs::metadata(&package).expect("meta").len().max(1000)
    );
}
