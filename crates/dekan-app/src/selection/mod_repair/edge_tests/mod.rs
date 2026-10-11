use super::tests::{fantome, game, shared, skin0, temp, wad_bytes};
use std::io::Write;

use dekan_wad::hash::wad_path_hash;
use dekan_wad::prop::serialize_prop_links;

use super::*;

fn logs() -> tracing::subscriber::DefaultGuard {
    tracing::subscriber::set_default(
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_test_writer()
            .finish(),
    )
}

fn skn_head(major: u16, minor: u16) -> Vec<u8> {
    let mut head = vec![0x33, 0x22, 0x11, 0x00];
    head.extend_from_slice(&major.to_le_bytes());
    head.extend_from_slice(&minor.to_le_bytes());
    head.resize(32, 0);
    head
}

#[test]
fn test_a_full_scan_with_logs_on_covers_every_report() {
    let _logs = logs();
    let dir = temp("logged_scan");
    let game_dir = game(&dir, &[shared(&[0, 1, 44])]);
    let champions = game_dir.join("DATA").join("FINAL").join("Champions");
    let mut entries = vec![(skin0(), serialize_prop_links(&[shared(&[0, 1, 44])], 3))];
    entries.push((
        wad_path_hash(&shared(&[0, 1, 44]).to_ascii_lowercase()),
        b"shared".to_vec(),
    ));
    entries.push((
        wad_path_hash("assets/characters/zed/zed.skn"),
        skn_head(4, 1),
    ));
    std::fs::write(champions.join("Zed.wad.client"), wad_bytes(&entries)).expect("game wad");

    let root = dir.join("custom_mods");
    let archive = root.join("skins").join("Zed.fantome");
    std::fs::create_dir_all(archive.parent().expect("parent")).expect("dir");
    let wad = wad_bytes(&[
        (skin0(), serialize_prop_links(&[shared(&[0, 1])], 3)),
        (
            wad_path_hash("assets/characters/zed/old.skn"),
            skn_head(2, 1),
        ),
    ]);
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&archive).expect("zip"));
    let options = zip::write::SimpleFileOptions::default();
    zip.start_file("META/info.json", options).expect("meta");
    zip.write_all(br#"{"Name":"Zed"}"#).expect("meta");
    zip.start_file("WAD/Zed.wad.client", options).expect("wad");
    zip.write_all(&wad).expect("wad");
    zip.finish().expect("finish");

    let package = root.join("skins").join("Zed.modpkg");
    std::fs::write(
        &package,
        crate::selection::mods_store::tests::modpkg_with_one_raw_chunk(
            "zed.wad.client",
            skin0(),
            &serialize_prop_links(&[shared(&[0, 1])], 3),
        ),
    )
    .expect("package");
    let state = dir.join("state");

    let report = repair_custom_mods(std::slice::from_ref(&root), &game_dir, &state, &|| false);
    assert_eq!(report.repaired, vec![archive.clone()]);
    assert!(restore_originals(std::slice::from_ref(&root), &state).restored == 1);
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: cleanup
}

#[test]
fn test_verdicts_that_cannot_be_saved_are_reported_and_the_scan_still_finishes() {
    let _logs = logs();
    let dir = temp("verdicts_blocked");
    let game_dir = game(&dir, &[shared(&[0, 1, 44])]);
    let root = dir.join("custom_mods");
    fantome(&root.join("skins").join("Zed.fantome"), &[shared(&[0, 1])]);
    let state = dir.join("state");
    std::fs::create_dir_all(state.join(VERDICTS_FILE).join("blocker"))
        .expect("a folder in the way");

    let report = repair_custom_mods(std::slice::from_ref(&root), &game_dir, &state, &|| false);
    assert!(report.ran);
    assert_eq!(report.repaired.len(), 1);
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: cleanup
}

#[test]
fn test_missing_roots_empty_mod_folders_and_unreadable_wads_are_handled() {
    let _logs = logs();
    let dir = temp("odd_inputs");
    let game_dir = game(&dir, &[shared(&[0, 1, 44])]);
    let root = dir.join("custom_mods");
    let empty = root.join("skins").join("Empty");
    std::fs::create_dir_all(empty.join("META")).expect("meta");
    std::fs::write(empty.join("META").join("info.json"), br#"{"Name":"Empty"}"#).expect("info");
    std::fs::create_dir_all(empty.join("WAD").join("nothing_inside")).expect("wad dir");
    let broken = root.join("skins").join("Broken.fantome");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&broken).expect("zip"));
    let options = zip::write::SimpleFileOptions::default();
    zip.start_file("META/info.json", options).expect("meta");
    zip.write_all(br#"{"Name":"Broken"}"#).expect("meta");
    zip.start_file("WAD/Zed.wad.client", options).expect("wad");
    zip.write_all(b"not a wad").expect("wad");
    zip.finish().expect("finish");

    let report = repair_custom_mods(
        &[root.clone(), dir.join("missing_root")],
        &game_dir,
        &dir.join("state"),
        &|| false,
    );
    assert!(report.ran);
    assert_eq!(
        report.checked, 1,
        "the broken archive is tried, the empty folder has nothing to stamp"
    );
    assert!(report.repaired.is_empty() && report.incompatible.is_empty());
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: cleanup
}

#[test]
fn test_an_archive_that_cannot_be_replaced_keeps_the_original_and_no_partial_file() {
    let dir = temp("archive_readonly");
    let game_dir = game(&dir, &[shared(&[0, 1, 44])]);
    let root = dir.join("custom_mods");
    let archive = root.join("skins").join("Zed.fantome");
    fantome(&archive, &[shared(&[0, 1])]);
    let original = std::fs::read(&archive).expect("read");
    let mut permissions = std::fs::metadata(&archive).expect("meta").permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&archive, permissions.clone()).expect("readonly");

    let report = repair_custom_mods(
        std::slice::from_ref(&root),
        &game_dir,
        &dir.join("state"),
        &|| false,
    );
    assert!(report.repaired.is_empty());
    assert_eq!(std::fs::read(&archive).expect("read"), original);
    assert!(!PathBuf::from(format!("{}.partial", archive.display())).exists());
    #[allow(clippy::permissions_set_readonly_false)]
    permissions.set_readonly(false);
    std::fs::set_permissions(&archive, permissions).expect("writable");
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: cleanup
}

#[test]
fn test_a_broken_game_archive_does_not_stop_the_scan() {
    let dir = temp("index_fails");
    let game_dir = game(&dir, &[shared(&[0, 1, 44])]);
    std::fs::write(
        game_dir
            .join("DATA")
            .join("FINAL")
            .join("Champions")
            .join("Broken.wad.client"),
        b"not a wad",
    )
    .expect("broken game wad");
    let root = dir.join("custom_mods");
    fantome(&root.join("skins").join("Zed.fantome"), &[shared(&[0, 1])]);

    let report = repair_custom_mods(
        std::slice::from_ref(&root),
        &game_dir,
        &dir.join("state"),
        &|| false,
    );
    assert!(report.ran, "a broken game archive is skipped, not fatal");
    assert_eq!(report.repaired.len(), 1);
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: cleanup
}
