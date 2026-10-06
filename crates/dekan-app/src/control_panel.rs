use std::path::PathBuf;

use dekan_core::party::PartyStatus;
use dekan_inject::ltk_host::{DllSupport, dll_support};
use dekan_platform::i18n::{Text, fill};
use dekan_platform::panel::{PanelCheck, PanelSnapshot};

#[derive(Debug, Clone)]
pub struct Facts {
    pub status: String,
    pub party_line: String,
    pub in_room: bool,
    pub auto_accept: bool,
    pub random_skin: bool,
    pub autostart: bool,
    pub tools_present: bool,
    pub game_found: bool,
    pub lcu_connected: bool,
    pub game_build: Option<u32>,
    pub now_secs: u64,
    pub elevated: bool,
    pub update: Option<String>,
}

#[must_use]
pub fn party_line(status: &PartyStatus, hosting: bool, text: &Text) -> (String, bool) {
    match status {
        PartyStatus::Off => (text.party_off.to_owned(), false),
        PartyStatus::Unavailable { .. } => (text.party_unavailable.to_owned(), false),
        PartyStatus::Connecting if hosting => (text.party_created_connecting.to_owned(), true),
        PartyStatus::Connecting => (text.party_connecting.to_owned(), true),
        PartyStatus::Connected { members } => {
            let line = if hosting {
                text.party_created_in_room
            } else {
                text.party_in_room
            };
            (fill(line, "n", &members.to_string()), true)
        }
        PartyStatus::Error { .. } => (text.party_reconnecting.to_owned(), true),
    }
}

#[must_use]
pub fn snapshot(facts: &Facts, text: &Text) -> PanelSnapshot {
    let check = |label: &str, ok: bool, detail: String| PanelCheck {
        label: label.to_owned(),
        ok,
        detail,
    };
    let dll = match facts
        .game_build
        .map(|stamp| dll_support(stamp, facts.now_secs))
    {
        Some(DllSupport::Supported) => check(text.check_dll, true, text.detail_ok.to_owned()),
        Some(DllSupport::SupportedUntilNextPatch { days_left }) => check(
            text.check_dll,
            false,
            fill(
                text.detail_dll_days_left,
                "n",
                &days_left.max(0).to_string(),
            ),
        ),
        Some(DllSupport::Refused) => {
            check(text.check_dll, false, text.detail_dll_refused.to_owned())
        }
        None => check(text.check_dll, false, text.detail_dll_unknown.to_owned()),
    };
    PanelSnapshot {
        status: facts.status.clone(),
        party_line: facts.party_line.clone(),
        in_room: facts.in_room,
        auto_accept: facts.auto_accept,
        random_skin: facts.random_skin,
        autostart: facts.autostart,
        update_line: facts.update.as_deref().map(|latest| {
            fill(
                &fill(text.panel_update_line, "version", latest),
                "current",
                dekan_platform::version::display_version(),
            )
        }),
        checks: vec![
            check(
                text.check_injector,
                facts.tools_present,
                if facts.tools_present {
                    text.detail_ok
                } else {
                    text.detail_injector_missing
                }
                .to_owned(),
            ),
            check(
                text.check_game,
                facts.game_found,
                if facts.game_found {
                    text.detail_ok
                } else {
                    text.detail_game_missing
                }
                .to_owned(),
            ),
            check(
                text.check_client,
                facts.lcu_connected,
                if facts.lcu_connected {
                    text.detail_client_connected
                } else {
                    text.detail_client_waiting
                }
                .to_owned(),
            ),
            dll,
            check(
                text.check_privileges,
                true,
                if facts.elevated {
                    text.detail_elevated
                } else {
                    text.detail_not_elevated
                }
                .to_owned(),
            ),
        ],
    }
}

const DIAGNOSTIC_LOG_DAYS: u64 = 3;
const DIAGNOSTIC_MAX_BYTES: u64 = 256 * 1024 * 1024;

pub fn export_diagnostics(
    logs_dir: &std::path::Path,
    now: std::time::SystemTime,
    screenshots: &[PathBuf],
) -> Result<PathBuf, String> {
    use std::io::Write;

    let cutoff = now
        .checked_sub(std::time::Duration::from_secs(
            DIAGNOSTIC_LOG_DAYS * 24 * 60 * 60,
        ))
        .unwrap_or(std::time::UNIX_EPOCH);
    let mut logs: Vec<PathBuf> = std::fs::read_dir(logs_dir)
        .map_err(|e| format!("logs folder unreadable: {e}"))?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("dekan.log"))
                && std::fs::metadata(path)
                    .and_then(|m| m.modified())
                    .is_ok_and(|t| t >= cutoff)
        })
        .collect();
    logs.sort();
    if logs.is_empty() {
        return Err("no Dekan log from the last days".into());
    }
    let mut files: Vec<(String, PathBuf)> = logs
        .iter()
        .filter_map(|p| Some((p.file_name()?.to_str()?.to_owned(), p.clone())))
        .collect();
    if let Some(data_dir) = logs_dir.parent() {
        let overlay = data_dir.join("overlay_manifest.json");
        if overlay.is_file() {
            files.push(("manifests/overlay_manifest.json".into(), overlay));
        }
        if let Ok(mods) = std::fs::read_dir(data_dir.join("mods")) {
            let mut generated: Vec<(String, PathBuf)> = mods
                .flatten()
                .filter_map(|m| {
                    let manifest = m.path().join("META").join("manifest.json");
                    let name = m.file_name().to_str()?.to_owned();
                    manifest
                        .is_file()
                        .then(|| (format!("manifests/{name}.json"), manifest))
                })
                .collect();
            generated.sort();
            files.extend(generated);
        }
    }
    files.extend(screenshots.iter().filter_map(|shot| {
        Some((
            format!("screenshots/{}", shot.file_name()?.to_str()?),
            shot.clone(),
        ))
    }));
    let stamp = now
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let target = logs_dir.join(format!("dekan-diagnostics-{stamp}.zip"));
    let partial = logs_dir.join(format!("dekan-diagnostics-{stamp}.zip.partial"));
    let written = (|| -> Result<(), String> {
        let file = std::fs::File::create(&partial).map_err(|e| e.to_string())?;
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        let mut total = 0u64;
        for (name, path) in &files {
            let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
            total += bytes.len() as u64;
            if total > DIAGNOSTIC_MAX_BYTES {
                return Err("logs larger than the diagnostic limit".into());
            }
            zip.start_file(name.as_str(), options)
                .map_err(|e| e.to_string())?;
            zip.write_all(&bytes).map_err(|e| e.to_string())?;
        }
        zip.finish().map_err(|e| e.to_string())?;
        Ok(())
    })();
    if let Err(e) = written {
        let _ = std::fs::remove_file(&partial); // ignore-ok: the export error is what gets reported
        return Err(e);
    }
    std::fs::rename(&partial, &target).map_err(|e| {
        let _ = std::fs::remove_file(&partial); // ignore-ok: the rename error is what gets reported
        e.to_string()
    })?;
    Ok(target)
}

#[must_use]
pub fn game_found(configured: &std::path::Path) -> bool {
    dekan_platform::paths::is_valid_game_dir(configured)
        || dekan_platform::paths::discover_game_dir().is_some()
}

#[must_use]
pub fn tools_present(files: &[PathBuf]) -> bool {
    files.iter().all(|file| file.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dekan_platform::i18n::Language;

    fn facts() -> Facts {
        Facts {
            status: "s".into(),
            party_line: "p".into(),
            in_room: false,
            auto_accept: false,
            random_skin: true,
            autostart: false,
            tools_present: true,
            game_found: true,
            lcu_connected: true,
            game_build: Some(1_790_205_875),
            now_secs: 1_790_700_000,
            elevated: false,
            update: None,
        }
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
}
