use super::tests::{fantome, game, shared, skin0, temp, wad_bytes};

use dekan_wad::prop::serialize_prop_links;

use super::*;

#[test]
fn test_restoring_originals_puts_them_back_and_keeps_them_unrepaired() {
    let dir = temp("restore");
    let game_dir = game(&dir, &[shared(&[0, 1, 44])]);
    let root = dir.join("custom_mods");
    let archive = root.join("skins").join("Zed.fantome");
    fantome(&archive, &[shared(&[0, 1])]);
    let original = std::fs::read(&archive).expect("read");
    let state = dir.join("state");

    let first = repair_custom_mods(std::slice::from_ref(&root), &game_dir, &state, &|| false);
    assert_eq!(first.repaired, vec![archive.clone()]);

    let outcome = restore_originals(std::slice::from_ref(&root), &state);
    assert_eq!(
        outcome,
        RestoreOutcome {
            busy: false,
            restored: 1,
            failed: Vec::new()
        }
    );
    assert_eq!(std::fs::read(&archive).expect("read"), original);
    assert!(
        !state
            .join(ORIGINALS_DIR)
            .join("skins")
            .join("Zed.fantome")
            .exists()
    );

    let again = repair_custom_mods(std::slice::from_ref(&root), &game_dir, &state, &|| false);
    assert_eq!(again.kept_original, 1);
    assert!(again.repaired.is_empty());
    assert_eq!(std::fs::read(&archive).expect("read"), original);

    fantome(&archive, &[shared(&[0, 1]), shared(&[0, 1, 2])]);
    let replaced = repair_custom_mods(std::slice::from_ref(&root), &game_dir, &state, &|| false);
    assert_eq!(replaced.kept_original, 0);
    assert_eq!(replaced.checked, 1);
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: cleanup
}

#[test]
fn test_a_mod_replaced_after_its_repair_is_never_overwritten_by_restore() {
    let dir = temp("restore_replaced");
    let game_dir = game(&dir, &[shared(&[0, 1, 44])]);
    let root = dir.join("custom_mods");
    let archive = root.join("skins").join("Zed.fantome");
    fantome(&archive, &[shared(&[0, 1])]);
    let state = dir.join("state");
    repair_custom_mods(std::slice::from_ref(&root), &game_dir, &state, &|| false);

    fantome(&archive, &[shared(&[0, 1, 44]), shared(&[0, 1, 44])]);
    let newer = std::fs::read(&archive).expect("read");
    let outcome = restore_originals(std::slice::from_ref(&root), &state);
    assert_eq!(outcome.restored, 0);
    assert_eq!(std::fs::read(&archive).expect("read"), newer);
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: cleanup
}

#[test]
fn test_a_kept_original_stays_kept_after_a_new_game_version() {
    let dir = temp("kept_patch");
    let game_dir = game(&dir, &[shared(&[0, 1, 44])]);
    let root = dir.join("custom_mods");
    let archive = root.join("skins").join("Zed.fantome");
    fantome(&archive, &[shared(&[0, 1])]);
    let state = dir.join("state");
    repair_custom_mods(std::slice::from_ref(&root), &game_dir, &state, &|| false);
    assert_eq!(
        restore_originals(std::slice::from_ref(&root), &state).restored,
        1
    );

    std::fs::write(
        game_dir.join(dekan_platform::game_version::GAME_EXES[0]),
        b"patched exe",
    )
    .expect("exe");
    let report = repair_custom_mods(std::slice::from_ref(&root), &game_dir, &state, &|| false);
    assert_eq!(report.kept_original, 1);
    assert!(report.repaired.is_empty());
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: cleanup
}

#[test]
fn test_restore_waits_for_a_running_check_and_a_check_waits_for_restore() {
    let dir = temp("restore_busy");
    let game_dir = game(&dir, &[shared(&[0, 1, 44])]);
    let root = dir.join("custom_mods");
    fantome(&root.join("skins").join("Zed.fantome"), &[shared(&[0, 1])]);
    let state = dir.join("state");

    let guard = ScanGuard::try_enter(&state).expect("guard");
    assert!(restore_originals(std::slice::from_ref(&root), &state).busy);
    assert!(!repair_custom_mods(std::slice::from_ref(&root), &game_dir, &state, &|| false).ran);
    drop(guard);
    assert!(repair_custom_mods(std::slice::from_ref(&root), &game_dir, &state, &|| false).ran);
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: cleanup
}

#[test]
fn test_a_restore_that_cannot_put_a_file_back_reports_it_and_keeps_the_rest_consistent() {
    let dir = temp("restore_fail");
    let game_dir = game(&dir, &[shared(&[0, 1, 44])]);
    let root = dir.join("custom_mods");
    let mod_dir = root.join("skins").join("ZedFolder");
    std::fs::create_dir_all(mod_dir.join("META")).expect("meta");
    std::fs::write(
        mod_dir.join("META").join("info.json"),
        br#"{"Name":"Zed Folder"}"#,
    )
    .expect("info");
    let wad = mod_dir.join("WAD").join("Zed.wad.client");
    std::fs::create_dir_all(wad.parent().expect("parent")).expect("dir");
    std::fs::write(
        &wad,
        wad_bytes(&[(skin0(), serialize_prop_links(&[shared(&[0, 1])], 3))]),
    )
    .expect("wad");
    let state = dir.join("state");
    repair_custom_mods(std::slice::from_ref(&root), &game_dir, &state, &|| false);

    let locked = {
        use std::os::windows::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&wad)
            .expect("lock")
    };
    let outcome = restore_originals(std::slice::from_ref(&root), &state);
    assert_eq!(outcome.restored, 0);
    assert_eq!(outcome.failed.len(), 1, "{outcome:?}");
    drop(locked);
    assert!(wad.is_file(), "the repaired file is never lost");
    assert_eq!(
        restore_originals(std::slice::from_ref(&root), &state).restored,
        1
    );
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: cleanup
}

#[test]
fn test_restore_skips_compatible_mods_and_mods_whose_backup_is_gone() {
    let dir = temp("restore_skips");
    let game_dir = game(&dir, &[shared(&[0, 1, 44])]);
    let root = dir.join("custom_mods");
    fantome(
        &root.join("skins").join("Fine.fantome"),
        &[shared(&[0, 1, 44])],
    );
    let archive = root.join("skins").join("Zed.fantome");
    fantome(&archive, &[shared(&[0, 1])]);
    let mod_dir = root.join("skins").join("ZedFolder");
    std::fs::create_dir_all(mod_dir.join("META")).expect("meta");
    std::fs::write(
        mod_dir.join("META").join("info.json"),
        br#"{"Name":"Folder"}"#,
    )
    .expect("info");
    let folder_wad = mod_dir.join("WAD").join("Zed.wad.client");
    std::fs::create_dir_all(folder_wad.parent().expect("parent")).expect("dir");
    std::fs::write(
        &folder_wad,
        wad_bytes(&[(skin0(), serialize_prop_links(&[shared(&[0, 1])], 3))]),
    )
    .expect("wad");
    let state = dir.join("state");
    repair_custom_mods(std::slice::from_ref(&root), &game_dir, &state, &|| false);

    std::fs::remove_file(state.join(ORIGINALS_DIR).join("skins").join("Zed.fantome"))
        .expect("backup");
    std::fs::remove_file(
        state
            .join(ORIGINALS_DIR)
            .join("skins")
            .join("ZedFolder")
            .join("WAD")
            .join("Zed.wad.client"),
    )
    .expect("folder backup");
    let outcome = restore_originals(std::slice::from_ref(&root), &state);
    assert_eq!(outcome.restored, 0);
    assert!(outcome.failed.is_empty());
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: cleanup
}
