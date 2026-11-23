use super::*;
use dekan_platform::i18n::Language;

fn facts() -> Facts {
    Facts {
        status: "s".into(),
        party_line: "p".into(),
        in_room: false,
        auto_accept: false,
        injection: true,
        random_skin: true,
        light_loading: true,
        autostart: false,
        tools_present: true,
        manager: ManagerStatus::Ready {
            version: Some("1.33.0".into()),
        },
        game_found: true,
        lcu_connected: true,
        elevated: false,
        update: None,
        custom_mods: None,
        stats: dekan_platform::panel::PanelStats::default(),
        repair: dekan_platform::panel::PanelRepair::default(),
    }
}

#[test]
fn test_the_custom_mods_row_shows_once_the_startup_scan_ran() {
    let text = Language::English.text();
    assert!(
        snapshot(&facts(), text)
            .checks
            .iter()
            .all(|c| c.label != text.check_custom_mods)
    );
    let scanned = Facts {
        custom_mods: Some(crate::mod_repair::ScanSummary {
            repaired: 2,
            refused: 1,
        }),
        ..facts()
    };
    let row = snapshot(&scanned, text)
        .checks
        .into_iter()
        .find(|c| c.label == text.check_custom_mods)
        .expect("custom mods row");
    assert!(!row.ok);
    assert_eq!(row.detail, "2 adjusted to the patch · 1 left out");
}

#[test]
fn test_a_newer_release_is_shown_with_both_versions() {
    let text = Language::English.text();
    assert_eq!(snapshot(&facts(), text).update_line, None);
    let line = snapshot(
        &Facts {
            update: Some("9.4".into()),
            ..facts()
        },
        text,
    )
    .update_line
    .expect("update line");
    assert!(line.contains("9.4"), "{line}");
    assert!(
        line.contains(dekan_platform::version::display_version()),
        "{line}"
    );
    assert!(!line.contains('{'), "{line}");
}

#[test]
fn test_a_healthy_install_reports_every_check_ok() {
    let text = Language::English.text();
    let snapshot = snapshot(&facts(), text);
    let failing: Vec<&str> = snapshot
        .checks
        .iter()
        .filter(|c| !c.ok)
        .map(|c| c.label.as_str())
        .collect();
    assert!(failing.is_empty(), "{failing:?}");
}

#[test]
fn test_diagnostics_zip_holds_recent_logs_and_every_manifest() {
    let root = std::env::temp_dir().join(format!("dekan_diag_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root); // ignore-ok: fixture may not exist yet
    let dir = root.join("logs");
    std::fs::create_dir_all(&dir).expect("dir");
    std::fs::write(dir.join("dekan.log.2026-09-30"), b"today").expect("log");
    std::fs::write(dir.join("notes.txt"), b"not a log").expect("other");
    std::fs::write(root.join("overlay_manifest.json"), b"{}").expect("overlay manifest");
    let meta = root.join("mods").join("std_zed_70").join("META");
    std::fs::create_dir_all(&meta).expect("meta");
    std::fs::write(meta.join("manifest.json"), b"{}").expect("skin manifest");
    let zip_path = export_diagnostics(&dir, std::time::SystemTime::now(), &[]).expect("export");
    let mut archive =
        zip::ZipArchive::new(std::fs::File::open(&zip_path).expect("zip")).expect("archive");
    let names: Vec<String> = (0..archive.len())
        .map(|i| {
            archive
                .by_index(i)
                .expect("entry")
                .name()
                .expect("name")
                .into_owned()
        })
        .collect();
    assert_eq!(
        names,
        vec![
            "dekan.log.2026-09-30".to_owned(),
            "manifests/overlay_manifest.json".to_owned(),
            "manifests/std_zed_70.json".to_owned(),
        ]
    );
    assert!(!dir.join(format!("{}.partial", zip_path.display())).exists());
    let _ = std::fs::remove_dir_all(&root); // ignore-ok: fixture cleanup
}

#[test]
fn test_diagnostics_keep_screenshots_as_they_are_and_compress_the_logs() {
    let root = std::env::temp_dir().join(format!("dekan_diag_shot_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root); // ignore-ok: fixture may not exist yet
    let dir = root.join("logs");
    std::fs::create_dir_all(&dir).expect("dir");
    std::fs::write(dir.join("dekan.log.2026-09-30"), b"today").expect("log");
    let shot = root.join("Screen01.png");
    std::fs::write(&shot, [0x89, b'P', b'N', b'G', 1, 2, 3]).expect("shot");
    let zip_path = export_diagnostics(&dir, std::time::SystemTime::now(), &[shot]).expect("export");
    let mut archive =
        zip::ZipArchive::new(std::fs::File::open(&zip_path).expect("zip")).expect("archive");
    let image = archive.by_name("screenshots/Screen01.png").expect("shot");
    assert_eq!(image.compression(), zip::CompressionMethod::Stored);
    assert_eq!(image.size(), 7);
    drop(image);
    let log = archive.by_name("dekan.log.2026-09-30").expect("log");
    assert_eq!(log.compression(), zip::CompressionMethod::Deflated);
    drop(log);
    let _ = std::fs::remove_dir_all(&root); // ignore-ok: fixture cleanup
}

#[test]
fn test_diagnostics_refuse_an_empty_logs_folder() {
    let dir = std::env::temp_dir().join(format!("dekan_diag_empty_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: fixture may not exist yet
    std::fs::create_dir_all(&dir).expect("dir");
    assert!(export_diagnostics(&dir, std::time::SystemTime::now(), &[]).is_err());
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: fixture cleanup
}

#[test]
fn test_tools_are_present_only_when_every_file_exists() {
    let dir = std::env::temp_dir().join(format!("dekan_panel_tools_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: fixture may not exist yet
    std::fs::create_dir_all(&dir).expect("fixture dir");
    let host = dir.join("dekan_launcher.exe");
    let dll = dir.join("dekan_bridge.dll");
    std::fs::write(&host, b"host").expect("host");
    assert!(!tools_present(&[host.clone(), dll.clone()]));
    std::fs::write(&dll, b"dll").expect("dll");
    assert!(tools_present(&[host, dll]));
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: fixture cleanup
}

#[test]
fn test_a_room_you_created_says_so_until_you_leave() {
    for language in [Language::Turkish, Language::English] {
        let text = language.text();
        let connected = PartyStatus::Connected { members: 1 };

        let (created, in_room) = party_line(&connected, true, text);
        assert_eq!(created, fill(text.party_created_in_room, "n", "1"));
        assert!(in_room);
        assert_ne!(created, fill(text.party_in_room, "n", "1"));

        let (joined, _) = party_line(&connected, false, text);
        assert_eq!(joined, fill(text.party_in_room, "n", "1"));

        let (connecting, _) = party_line(&PartyStatus::Connecting, true, text);
        assert_eq!(connecting, text.party_created_connecting);

        let (left, in_room) = party_line(&PartyStatus::Off, true, text);
        assert_eq!(left, text.party_off);
        assert!(!in_room);
    }
}

#[test]
fn test_missing_pieces_are_reported() {
    let text = Language::Turkish.text();
    let snap = snapshot(
        &Facts {
            tools_present: false,
            manager: ManagerStatus::Missing,
            game_found: false,
            lcu_connected: false,
            ..facts()
        },
        text,
    );
    let details: Vec<&str> = snap.checks.iter().map(|c| c.detail.as_str()).collect();
    assert!(details.contains(&text.detail_injector_missing));
    assert!(details.contains(&text.detail_manager_missing));
    assert!(details.contains(&text.detail_game_missing));
    assert!(details.contains(&text.detail_client_waiting));

    let outdated = snapshot(
        &Facts {
            manager: ManagerStatus::Outdated {
                found: "1.29.0".into(),
                required: "1.33.0".into(),
            },
            ..facts()
        },
        text,
    );
    let row = outdated
        .checks
        .into_iter()
        .find(|c| c.label == text.check_manager)
        .expect("manager check row");
    assert!(!row.ok);
    assert!(row.detail.contains("1.29.0") && row.detail.contains("1.33.0"));
}

#[test]
fn test_repair_state_prevents_concurrent_runs_and_transitions_cleanly() {
    let state = RepairState::default();
    assert!(state.begin_skins("running skins"));
    assert!(!state.begin_skins("duplicate skins"));
    let busy = state.snapshot();
    assert!(busy.skins_busy);
    assert_eq!(busy.state, REPAIR_STATE_BUSY);
    assert_eq!(busy.message, "running skins");

    state.finish_skins(true, "skins fixed".into());
    let done = state.snapshot();
    assert!(!done.skins_busy);
    assert_eq!(done.state, REPAIR_STATE_OK);
    assert_eq!(done.message, "skins fixed");

    assert!(state.begin_client("running client"));
    assert!(!state.begin_client("duplicate client"));
    state.finish_client(false, "file locked".into());
    let failed = state.snapshot();
    assert!(!failed.client_busy);
    assert_eq!(failed.state, REPAIR_STATE_ERROR);
    assert_eq!(failed.message, "file locked");
}

#[test]
fn test_repair_feedback_formatting_covers_all_languages_and_outcomes() {
    for lang in [Language::Turkish, Language::English] {
        let text = lang.text();
        let (ok, msg) = format_skins_feedback(
            &RepairSkinsReport {
                cleared_generated: 3,
                checked: 5,
                repaired: 2,
                incompatible: 0,
                game_found: true,
                totals: None,
            },
            text,
        );
        assert!(ok);
        assert!(msg.contains('3') && msg.contains('5') && msg.contains('2') && !msg.contains('{'));

        let (warn_ok, warn_msg) = format_skins_feedback(
            &RepairSkinsReport {
                cleared_generated: 1,
                checked: 2,
                repaired: 1,
                incompatible: 1,
                game_found: true,
                totals: None,
            },
            text,
        );
        assert!(!warn_ok);
        assert!(!warn_msg.contains('{'));

        let (no_game_ok, no_game_msg) = format_skins_feedback(
            &RepairSkinsReport {
                cleared_generated: 2,
                game_found: false,
                ..RepairSkinsReport::default()
            },
            text,
        );
        assert!(!no_game_ok);
        assert!(no_game_msg.contains('2') && !no_game_msg.contains('{'));

        let (client_ok, client_msg) = format_client_feedback(
            &RepairClientReport {
                cleared_caches: 4,
                stopped_processes: 1,
                reset_bridge: true,
                warnings: Vec::new(),
            },
            text,
        );
        assert!(client_ok);
        assert!(client_msg.contains('4') && !client_msg.contains('{'));

        let (partial_ok, partial_msg) = format_client_feedback(
            &RepairClientReport {
                cleared_caches: 2,
                warnings: vec!["overlay: busy".into()],
                ..RepairClientReport::default()
            },
            text,
        );
        assert!(!partial_ok);
        assert!(partial_msg.contains("overlay: busy") && !partial_msg.contains('{'));
    }
}
