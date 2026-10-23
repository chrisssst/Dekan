use crate::trigger::prepare::mods::*;
use std::path::{Path, PathBuf};

fn fixture(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("dekan_mod_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root); // ignore-ok: fixture may not exist yet
    std::fs::create_dir_all(&root).expect("fixture root");
    root
}

fn write_mod(archive: &Path) {
    let file = std::fs::File::create(archive).expect("archive");
    let mut writer = zip::ZipWriter::new(file);
    let options: zip::write::FileOptions<'_, '_, ()> = zip::write::FileOptions::default();
    writer
        .start_file("WAD/Ezreal.wad.client", options)
        .expect("wad");
    std::io::Write::write_all(&mut writer, b"RW\x03\x04").expect("wad body");
    writer.finish().expect("finish archive");
}

fn partial_of(target: &Path) -> PathBuf {
    let mut staging = target.as_os_str().to_owned();
    staging.push(".partial");
    PathBuf::from(staging)
}

#[test]
fn an_unchanged_archive_keeps_its_extraction() {
    let root = fixture("kept");
    let archive = root.join("1.fantome");
    write_mod(&archive);
    let target = root.join("mod");
    prepare_mod_directory(&archive, &target).expect("first extraction");
    let marker = target.join("marker");
    std::fs::write(&marker, b"x").expect("marker");

    prepare_mod_directory(&archive, &target).expect("second call");
    assert!(
        marker.is_file(),
        "a complete extraction of the same archive is reused"
    );
    let _ = std::fs::remove_dir_all(&root); // ignore-ok: fixture cleanup
}

#[test]
fn an_extraction_without_a_source_stamp_is_redone() {
    let root = fixture("unstamped");
    let archive = root.join("1.fantome");
    write_mod(&archive);
    let target = root.join("mod");
    std::fs::create_dir_all(target.join("WAD")).expect("old extraction");
    std::fs::write(target.join("WAD").join("Ezreal.wad.client"), b"old").expect("old wad");
    std::fs::write(target.join("marker"), b"x").expect("marker");

    prepare_mod_directory(&archive, &target).expect("extraction");
    assert!(!target.join("marker").exists());
    assert!(target.join(".dekan-source").is_file());
    let _ = std::fs::remove_dir_all(&root); // ignore-ok: fixture cleanup
}

#[test]
fn a_staging_folder_left_by_a_crash_is_cleared() {
    let root = fixture("crashed");
    let archive = root.join("1.fantome");
    write_mod(&archive);
    let target = root.join("mod");
    let staging = partial_of(&target);
    std::fs::create_dir_all(&staging).expect("leftover staging");
    std::fs::write(staging.join("stale"), b"x").expect("stale file");

    prepare_mod_directory(&archive, &target).expect("extraction");
    assert!(!staging.exists());
    assert!(target.join("WAD").join("Ezreal.wad.client").is_file());
    assert!(!target.join("stale").exists());
    let _ = std::fs::remove_dir_all(&root); // ignore-ok: fixture cleanup
}

#[test]
fn a_mod_folder_is_mirrored_and_stamped() {
    let root = fixture("folder");
    let source = root.join("source");
    std::fs::create_dir_all(source.join("WAD")).expect("source wad dir");
    std::fs::write(source.join("WAD").join("Ezreal.wad.client"), b"RW").expect("source wad");
    let target = root.join("mod");

    prepare_mod_directory(&source, &target).expect("mirror");
    assert!(target.join("WAD").join("Ezreal.wad.client").is_file());
    assert!(target.join(".dekan-source").is_file());
    let _ = std::fs::remove_dir_all(&root); // ignore-ok: fixture cleanup
}

#[test]
fn a_corrupt_or_missing_archive_is_an_error_and_leaves_no_mod() {
    let root = fixture("broken");
    let corrupt = root.join("corrupt.fantome");
    std::fs::write(&corrupt, b"not a zip archive").expect("corrupt archive");
    let target = root.join("mod");
    assert!(prepare_mod_directory(&corrupt, &target).is_err());
    assert!(!target.exists());

    let missing = prepare_mod_directory(&root.join("absent.fantome"), &target);
    assert_eq!(
        missing.map_err(|e| e.kind()),
        Err(std::io::ErrorKind::NotFound)
    );
    assert!(!target.exists());
    let _ = std::fs::remove_dir_all(&root); // ignore-ok: fixture cleanup
}
