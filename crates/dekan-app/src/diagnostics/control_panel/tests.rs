use super::*;
use dekan_platform::i18n::Language;

fn facts() -> Facts {
    Facts {
        status: "s".into(),
        party_line: "p".into(),
        in_room: false,
        auto_accept: false,
        random_skin: true,
        light_loading: true,
        autostart: false,
        tools_present: true,
        game_found: true,
        lcu_connected: true,
        game_build: Some(1_790_205_875),
        now_secs: u64::from(LIMIT) - 4 * 86_400 - 3_600,
        elevated: false,
        update: None,
        ltk: None,
        installed_dll: Some(InstalledDll {
            sha256: OLD_DLL.into(),
            build_limit: Some(LIMIT),
        }),
        custom_mods: None,
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

const LIMIT: u32 = 0x6ad4_6e70;
const OLD_DLL: &str = "07a43bf36a389eb00f6276e333bd7f2b95218f25a58e1e128ff4d2e4ab2dc99b";
const NEW_DLL: &str = "6d419057e6667994ba752ad0fb089b363db98267618644d7f7b6632441a21d74";

fn ltk(latest: &str, latest_trusted: bool, compatible: Option<(&str, &str)>) -> Option<LtkStatus> {
    Some(LtkStatus {
        latest: latest.into(),
        latest_trusted,
        compatible: compatible.map(|(version, _)| version.to_owned()),
        compatible_dll: compatible.map(|(_, dll)| dll.to_owned()),
    })
}

fn ltk_row<'a>(snapshot: &'a PanelSnapshot, text: &Text) -> &'a PanelCheck {
    snapshot
        .checks
        .iter()
        .find(|c| c.label == text.check_ltk)
        .expect("ltk row")
}

#[test]
fn test_the_ltk_check_names_the_signed_release_the_installed_dll_matches() {
    let text = Language::English.text();
    let unchecked = snapshot(&facts(), text);
    let row = ltk_row(&unchecked, text);
    assert!(row.ok);
    assert_eq!(row.detail, text.detail_ltk_unchecked);
    assert_eq!(unchecked.ltk_line, None);

    let current = snapshot(
        &Facts {
            ltk: ltk("1.26.1", true, Some(("1.26.1", OLD_DLL))),
            ..facts()
        },
        text,
    );
    let row = ltk_row(&current, text);
    assert!(row.ok);
    assert!(row.detail.contains("1.26.1"), "{}", row.detail);
    assert_eq!(current.ltk_line, None);
    assert_eq!(current.ltk_download, None);
}

#[test]
fn test_a_newer_signed_injector_offers_the_install_button() {
    let text = Language::English.text();
    let shown = snapshot(
        &Facts {
            ltk: ltk("1.27.0", true, Some(("1.27.0", NEW_DLL))),
            ..facts()
        },
        text,
    );
    let row = ltk_row(&shown, text);
    assert!(!row.ok);
    assert!(row.detail.contains("1.27.0"), "{}", row.detail);
    let line = shown.ltk_line.expect("ltk line");
    assert!(line.contains("1.27.0") && !line.contains('{'), "{line}");
    let button = shown.ltk_download.expect("install button");
    assert!(
        button.contains("1.27.0") && !button.contains('{'),
        "{button}"
    );
}

#[test]
fn test_an_unsigned_latest_release_warns_without_a_button() {
    let text = Language::English.text();
    let shown = snapshot(
        &Facts {
            ltk: ltk("1.28.0", false, Some(("1.26.1", OLD_DLL))),
            ..facts()
        },
        text,
    );
    let row = ltk_row(&shown, text);
    assert!(!row.ok);
    assert!(row.detail.contains("1.28.0"), "{}", row.detail);
    let line = shown.ltk_line.expect("ltk line");
    assert!(line.contains("1.28.0") && !line.contains('{'), "{line}");
    assert_eq!(shown.ltk_download, None);
}

#[test]
fn test_missing_tools_offer_the_newest_signed_release() {
    let text = Language::English.text();
    let missing = snapshot(
        &Facts {
            tools_present: false,
            installed_dll: None,
            ltk: ltk("1.28.0", false, Some(("1.27.0", NEW_DLL))),
            ..facts()
        },
        text,
    );
    let line = missing.ltk_line.expect("ltk line");
    assert!(line.contains("1.27.0"), "{line}");
    let button = missing.ltk_download.expect("install button");
    assert!(
        button.contains("1.27.0") && !button.contains('{'),
        "{button}"
    );

    let offline = snapshot(
        &Facts {
            tools_present: false,
            installed_dll: None,
            ..facts()
        },
        text,
    );
    let line = offline.ltk_line.expect("ltk line");
    assert!(line.contains(text.ltk_version_unknown), "{line}");
    assert_eq!(
        offline.ltk_download, None,
        "nothing to install without a known release"
    );
}

#[test]
fn test_the_dll_deadline_comes_from_the_installed_dll() {
    let text = Language::English.text();
    let unknown = snapshot(
        &Facts {
            installed_dll: Some(InstalledDll {
                sha256: OLD_DLL.into(),
                build_limit: None,
            }),
            ..facts()
        },
        text,
    );
    assert_eq!(unknown.checks[3].detail, text.detail_dll_unknown);

    let past = snapshot(
        &Facts {
            now_secs: u64::from(LIMIT) + 2 * 86_400,
            ..facts()
        },
        text,
    );
    assert_eq!(past.checks[3].detail, text.detail_dll_past_limit);
    assert!(!past.checks[3].ok);
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
fn test_a_healthy_install_reports_every_check_ok_but_the_dll_deadline() {
    let text = Language::English.text();
    let snapshot = snapshot(&facts(), text);
    let failing: Vec<&str> = snapshot
        .checks
        .iter()
        .filter(|c| !c.ok)
        .map(|c| c.label.as_str())
        .collect();
    assert_eq!(
        failing,
        vec![text.check_dll],
        "the deadline is days away, so it is shown"
    );
    assert!(snapshot.checks[3].detail.contains(" 4 "));
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
        .map(|i| archive.by_index(i).expect("entry").name().to_owned())
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
    let host = dir.join("ltk_patcher_host.exe");
    let dll = dir.join("ltk_patcher_dll.dll");
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
    let snapshot = snapshot(
        &Facts {
            tools_present: false,
            game_found: false,
            lcu_connected: false,
            game_build: None,
            ..facts()
        },
        text,
    );
    let details: Vec<&str> = snapshot.checks.iter().map(|c| c.detail.as_str()).collect();
    assert!(details.contains(&text.detail_injector_missing));
    assert!(details.contains(&text.detail_game_missing));
    assert!(details.contains(&text.detail_client_waiting));
    assert!(details.contains(&text.detail_dll_unknown));
}

#[test]
fn test_a_refused_build_is_reported_as_such() {
    let text = Language::English.text();
    let snapshot = snapshot(
        &Facts {
            game_build: Some(u32::MAX),
            ..facts()
        },
        text,
    );
    assert_eq!(snapshot.checks[3].detail, text.detail_dll_refused);
}
