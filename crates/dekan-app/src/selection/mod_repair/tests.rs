use std::io::{Read, Write};

use dekan_wad::hash::wad_path_hash;
use dekan_wad::prop::serialize_prop_links;
use dekan_wad::writer::{WadWriter, optimal_raw};

use super::*;

pub(super) fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "dekan_mod_repair_{name}_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: fixture may not exist yet
    std::fs::create_dir_all(&dir).expect("dir");
    dir
}

pub(super) fn shared(slots: &[u32]) -> String {
    let mut parts: Vec<String> = slots.iter().map(|n| format!("Skins_Skin{n}")).collect();
    parts.sort();
    format!(
        "DATA/Characters/Zed/Zed_Multi_Skins_{}.bin",
        parts.join("_")
    )
}

pub(super) fn wad_bytes(entries: &[(u64, Vec<u8>)]) -> Vec<u8> {
    let mut writer = WadWriter::default();
    for (hash, bytes) in entries {
        writer.insert(*hash, optimal_raw(bytes.clone()).expect("entry"));
    }
    writer.to_bytes().expect("wad")
}

pub(super) fn skin0() -> u64 {
    wad_path_hash("data/characters/zed/skins/skin0.bin")
}

pub(super) fn game(dir: &Path, links: &[String]) -> PathBuf {
    let game_dir = dir.join("Game");
    let champions = game_dir.join("DATA").join("FINAL").join("Champions");
    std::fs::create_dir_all(&champions).expect("dir");
    std::fs::write(
        game_dir.join(dekan_platform::game_version::GAME_EXES[0]),
        b"exe",
    )
    .expect("exe");
    let mut entries = vec![(skin0(), serialize_prop_links(links, 3))];
    for link in links {
        entries.push((
            wad_path_hash(&link.to_ascii_lowercase()),
            b"shared".to_vec(),
        ));
    }
    std::fs::write(champions.join("Zed.wad.client"), wad_bytes(&entries)).expect("game wad");
    game_dir
}

pub(super) fn fantome(path: &Path, links: &[String]) {
    std::fs::create_dir_all(path.parent().expect("parent")).expect("dir");
    let wad = wad_bytes(&[
        (skin0(), serialize_prop_links(links, 3)),
        (wad_path_hash("assets/zed/new.tex"), b"texture".to_vec()),
    ]);
    let mut zip = zip::ZipWriter::new(std::fs::File::create(path).expect("zip"));
    let options = zip::write::SimpleFileOptions::default();
    zip.start_file("META/info.json", options).expect("meta");
    zip.write_all(br#"{"Name":"Zed Test","Author":"t","Version":"1.0"}"#)
        .expect("meta");
    zip.start_file("WAD/Zed.wad.client", options).expect("wad");
    zip.write_all(&wad).expect("wad");
    zip.finish().expect("finish");
}

fn wad_in(archive: &Path) -> Vec<u8> {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(archive).expect("open")).expect("zip");
    let mut bytes = Vec::new();
    zip.by_name("WAD/Zed.wad.client")
        .expect("wad")
        .read_to_end(&mut bytes)
        .expect("read");
    bytes
}

#[test]
fn test_a_mod_broken_by_a_new_skin_is_repaired_once_with_a_backup() {
    let dir = temp("repaired");
    let game_dir = game(&dir, &[shared(&[0, 1, 44])]);
    let root = dir.join("custom_mods");
    let archive = root.join("skins").join("Zed.fantome");
    fantome(&archive, &[shared(&[0, 1])]);
    let original = std::fs::read(&archive).expect("read");
    let state = dir.join("state");

    let report = repair_custom_mods(std::slice::from_ref(&root), &game_dir, &state, &|| false);
    assert_eq!(report.repaired, vec![archive.clone()]);
    assert!(report.incompatible.is_empty());

    let fixed = dir.join("fixed.wad.client");
    std::fs::write(&fixed, wad_in(&archive)).expect("write");
    let game = get_or_index_game(&game_dir).expect("index");
    assert!(
        check(
            &fixed,
            &game_hash_set(&game),
            &std::collections::HashSet::new()
        )
        .expect("check")
        .is_compatible()
    );
    assert_eq!(
        std::fs::read(state.join(ORIGINALS_DIR).join("skins").join("Zed.fantome")).expect("backup"),
        original
    );
    assert!(!state.join(WORK_DIR).exists());

    let again = repair_custom_mods(std::slice::from_ref(&root), &game_dir, &state, &|| false);
    assert_eq!(again.checked, 0);
    assert_eq!(again.unchanged_since_last_run, 1);
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: cleanup
}

#[test]
fn test_a_mod_without_a_single_successor_is_left_untouched() {
    let dir = temp("incompatible");
    let game_dir = game(&dir, &[shared(&[0, 1, 44]), shared(&[0, 1, 45])]);
    let root = dir.join("custom_mods");
    let archive = root.join("skins").join("Zed.fantome");
    fantome(&archive, &[shared(&[0, 1])]);
    let original = std::fs::read(&archive).expect("read");
    let state = dir.join("state");

    let report = repair_custom_mods(std::slice::from_ref(&root), &game_dir, &state, &|| false);
    assert_eq!(report.incompatible, vec![archive.clone()]);
    assert_eq!(std::fs::read(&archive).expect("read"), original);
    assert!(!state.join(ORIGINALS_DIR).exists());
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: cleanup
}

#[test]
fn test_a_compatible_mod_is_never_rewritten() {
    let dir = temp("compatible");
    let link = shared(&[0, 1, 44]);
    let game_dir = game(&dir, std::slice::from_ref(&link));
    let root = dir.join("custom_mods");
    let archive = root.join("skins").join("Zed.fantome");
    fantome(&archive, &[link]);
    let original = std::fs::read(&archive).expect("read");

    let report = repair_custom_mods(
        std::slice::from_ref(&root),
        &game_dir,
        &dir.join("state"),
        &|| false,
    );
    assert_eq!(report.checked, 1);
    assert!(report.repaired.is_empty() && report.incompatible.is_empty());
    assert_eq!(std::fs::read(&archive).expect("read"), original);
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: cleanup
}

#[test]
fn test_a_new_game_version_checks_every_mod_again() {
    let dir = temp("new_patch");
    let link = shared(&[0, 1, 44]);
    let game_dir = game(&dir, std::slice::from_ref(&link));
    let root = dir.join("custom_mods");
    fantome(&root.join("skins").join("Zed.fantome"), &[link]);
    let state = dir.join("state");

    repair_custom_mods(std::slice::from_ref(&root), &game_dir, &state, &|| false);
    std::fs::write(
        game_dir.join(dekan_platform::game_version::GAME_EXES[0]),
        b"patched exe",
    )
    .expect("exe");
    let report = repair_custom_mods(std::slice::from_ref(&root), &game_dir, &state, &|| false);
    assert_eq!(report.checked, 1);
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: cleanup
}

#[test]
fn test_a_mod_folder_is_repaired_file_by_file_with_a_backup() {
    let dir = temp("folder_mod");
    let game_dir = game(&dir, &[shared(&[0, 1, 44])]);
    let root = dir.join("custom_mods");
    let mod_dir = root.join("skins").join("ZedFolder");
    std::fs::create_dir_all(mod_dir.join("META")).expect("meta");
    std::fs::write(
        mod_dir.join("META").join("info.json"),
        br#"{"Name":"Zed Folder"}"#,
    )
    .expect("info");
    std::fs::create_dir_all(mod_dir.join("WAD")).expect("wad dir");
    let wad = mod_dir.join("WAD").join("Zed.wad.client");
    let original = wad_bytes(&[(skin0(), serialize_prop_links(&[shared(&[0, 1])], 3))]);
    std::fs::write(&wad, &original).expect("wad");
    let state = dir.join("state");

    let report = repair_custom_mods(std::slice::from_ref(&root), &game_dir, &state, &|| false);
    assert_eq!(report.repaired, vec![mod_dir.clone()]);
    let index = get_or_index_game(&game_dir).expect("index");
    assert!(
        check(
            &wad,
            &game_hash_set(&index),
            &std::collections::HashSet::new()
        )
        .expect("check")
        .is_compatible()
    );
    assert_eq!(
        std::fs::read(
            state
                .join(ORIGINALS_DIR)
                .join("skins")
                .join("ZedFolder")
                .join("WAD")
                .join("Zed.wad.client")
        )
        .expect("backup"),
        original
    );
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: cleanup
}

#[test]
fn test_an_interrupted_scan_keeps_the_verdicts_it_did_not_reach() {
    let dir = temp("interrupted");
    let link = shared(&[0, 1, 44]);
    let game_dir = game(&dir, std::slice::from_ref(&link));
    let root = dir.join("custom_mods");
    fantome(&root.join("skins").join("Zed.fantome"), &[link]);
    let state = dir.join("state");

    repair_custom_mods(std::slice::from_ref(&root), &game_dir, &state, &|| false);
    let stopped = repair_custom_mods(std::slice::from_ref(&root), &game_dir, &state, &|| true);
    assert_eq!(stopped.checked, 0);
    let again = repair_custom_mods(std::slice::from_ref(&root), &game_dir, &state, &|| false);
    assert_eq!(again.unchanged_since_last_run, 1);
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: cleanup
}

#[test]
fn test_a_mod_package_is_checked_at_startup_but_never_rewritten() {
    let dir = temp("modpkg");
    let game_dir = game(&dir, &[shared(&[0, 1, 44])]);
    let root = dir.join("custom_mods");
    let package = root.join("skins").join("Zed.modpkg");
    std::fs::create_dir_all(package.parent().expect("parent")).expect("dir");
    let bytes = crate::selection::mods_store::tests::modpkg_with_one_raw_chunk(
        "zed.wad.client",
        skin0(),
        &serialize_prop_links(&[shared(&[0, 1])], 3),
    );
    std::fs::write(&package, &bytes).expect("package");
    let state = dir.join("state");

    let report = repair_custom_mods(std::slice::from_ref(&root), &game_dir, &state, &|| false);
    assert_eq!(report.checked, 1);
    assert!(report.repaired.is_empty() && report.incompatible.is_empty());
    assert_eq!(std::fs::read(&package).expect("read"), bytes);
    let saved = std::fs::read_to_string(state.join(VERDICTS_FILE)).expect("verdicts");
    assert!(saved.contains("RepairedOnEachInjection"), "{saved}");
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: cleanup
}

#[test]
fn test_the_panel_summary_counts_every_verdict_not_only_this_run() {
    let dir = temp("totals");
    let game_dir = game(&dir, &[shared(&[0, 1, 44])]);
    let root = dir.join("custom_mods");
    fantome(&root.join("skins").join("Zed.fantome"), &[shared(&[0, 1])]);
    fantome(&root.join("skins").join("Lost.fantome"), &[shared(&[0, 5])]);
    let state = dir.join("state");

    let first = repair_custom_mods(std::slice::from_ref(&root), &game_dir, &state, &|| false);
    assert_eq!(
        first.this_run(),
        ScanSummary {
            repaired: 1,
            refused: 1
        }
    );
    let second = repair_custom_mods(std::slice::from_ref(&root), &game_dir, &state, &|| false);
    assert_eq!(
        second.this_run(),
        ScanSummary {
            repaired: 0,
            refused: 0
        }
    );
    assert_eq!(
        second.totals,
        Some(ScanSummary {
            repaired: 1,
            refused: 1
        })
    );

    let notice = CustomModsNotice::default();
    assert_eq!(notice.summary(), None);
    notice.set(ScanSummary {
        repaired: 1,
        refused: 1,
    });
    assert_eq!(
        notice.clone().summary(),
        Some(ScanSummary {
            repaired: 1,
            refused: 1
        })
    );
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: cleanup
}

#[test]
fn test_without_the_game_executable_the_scan_does_not_run_or_count() {
    let dir = temp("no_exe");
    let game_dir = game(&dir, &[shared(&[0, 1, 44])]);
    std::fs::remove_file(game_dir.join(dekan_platform::game_version::GAME_EXES[0])).expect("exe");
    let root = dir.join("custom_mods");
    fantome(&root.join("skins").join("Zed.fantome"), &[shared(&[0, 1])]);

    let report = repair_custom_mods(
        std::slice::from_ref(&root),
        &game_dir,
        &dir.join("state"),
        &|| false,
    );
    assert!(!report.ran);
    assert_eq!(report.totals, None);
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: cleanup
}
