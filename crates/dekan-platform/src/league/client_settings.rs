use std::path::{Path, PathBuf};

use crate::error::PlatformError;

const SETTINGS_FILE: &str = "LeagueClientSettings.yaml";
const INDENT: &str = "    ";

#[must_use]
pub fn settings_path(game_dir: &Path) -> Option<PathBuf> {
    Some(game_dir.parent()?.join("Config").join(SETTINGS_FILE))
}

pub fn disable_crash_reporting(game_dir: &Path) -> Result<bool, PlatformError> {
    let path = settings_path(game_dir).ok_or_else(|| {
        PlatformError::Path(format!("'{}' has no install root", game_dir.display()))
    })?;
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(source) => {
            return Err(PlatformError::Io {
                context: format!("reading '{}'", path.display()),
                source,
            });
        }
    };
    match with_crash_reporting_off(&text) {
        Some(edited) => crate::fs::atomic_write(&path, edited.as_bytes(), true).map(|()| true),
        None => Ok(false),
    }
}

#[must_use]
pub fn with_crash_reporting_off(text: &str) -> Option<String> {
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let Some(install) = find_key(&lines, 0, lines.len(), 0, "install") else {
        lines.push("install:".into());
        lines.push(format!("{INDENT}crash_reporting:"));
        lines.push(format!("{INDENT}{INDENT}enabled: false"));
        return Some(join(&lines, newline));
    };
    let install_end = block_end(&lines, install);
    let child = child_indent(&lines, install, install_end);
    let Some(crash) = find_key(&lines, install + 1, install_end, child, "crash_reporting") else {
        lines.insert(
            install + 1,
            format!("{}crash_reporting:", " ".repeat(child)),
        );
        lines.insert(
            install + 2,
            format!("{}enabled: false", " ".repeat(child * 2)),
        );
        return Some(join(&lines, newline));
    };
    let crash_end = block_end(&lines, crash);
    let grandchild = child_indent(&lines, crash, crash_end);
    match find_key(&lines, crash + 1, crash_end, grandchild, "enabled") {
        Some(enabled) => {
            let value = lines[enabled].split_once(':').map(|(_, v)| v.trim());
            if value == Some("false") {
                return None;
            }
            lines[enabled] = format!("{}enabled: false", " ".repeat(grandchild));
        }
        None => lines.insert(
            crash + 1,
            format!("{}enabled: false", " ".repeat(grandchild)),
        ),
    }
    Some(join(&lines, newline))
}

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start_matches(' ').len()
}

fn is_content(line: &str) -> bool {
    let trimmed = line.trim();
    !trimmed.is_empty() && !trimmed.starts_with('#')
}

fn find_key(lines: &[String], from: usize, to: usize, indent: usize, key: &str) -> Option<usize> {
    (from..to).find(|&i| {
        let line = &lines[i];
        indent_of(line) == indent
            && line
                .trim_start()
                .strip_prefix(key)
                .is_some_and(|rest| rest.starts_with(':'))
    })
}

fn block_end(lines: &[String], parent: usize) -> usize {
    let indent = indent_of(&lines[parent]);
    (parent + 1..lines.len())
        .find(|&i| is_content(&lines[i]) && indent_of(&lines[i]) <= indent)
        .unwrap_or(lines.len())
}

fn child_indent(lines: &[String], parent: usize, end: usize) -> usize {
    (parent + 1..end)
        .find(|&i| is_content(&lines[i]))
        .map_or(indent_of(&lines[parent]) + INDENT.len(), |i| {
            indent_of(&lines[i])
        })
}

fn join(lines: &[String], newline: &str) -> String {
    let mut out = lines.join(newline);
    out.push_str(newline);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLIENT: &str = "install:\r\n    crash_reporting:\r\n        enabled: true\r\n        type: \"crashpad\"\r\n    game-settings:\r\n        modified: false\r\n";

    #[test]
    fn only_the_enabled_line_changes_and_line_endings_are_kept() {
        let edited = with_crash_reporting_off(CLIENT).expect("edited");
        assert_eq!(edited, CLIENT.replace("enabled: true", "enabled: false"));
    }

    #[test]
    fn an_already_disabled_file_is_left_alone() {
        let off = CLIENT.replace("enabled: true", "enabled: false");
        assert_eq!(with_crash_reporting_off(&off), None);
    }

    #[test]
    fn missing_sections_are_created_at_the_files_indentation() {
        let no_enabled = "install:\n  crash_reporting:\n    type: \"crashpad\"\n  globals:\n    locale: \"en_US\"\n";
        assert_eq!(
            with_crash_reporting_off(no_enabled).as_deref(),
            Some(
                "install:\n  crash_reporting:\n    enabled: false\n    type: \"crashpad\"\n  globals:\n    locale: \"en_US\"\n"
            )
        );
        let no_section = "install:\n  globals:\n    locale: \"en_US\"\n";
        assert_eq!(
            with_crash_reporting_off(no_section).as_deref(),
            Some(
                "install:\n  crash_reporting:\n    enabled: false\n  globals:\n    locale: \"en_US\"\n"
            )
        );
        assert_eq!(
            with_crash_reporting_off("").as_deref(),
            Some("install:\n    crash_reporting:\n        enabled: false\n")
        );
    }

    #[test]
    fn keys_with_the_same_name_elsewhere_are_not_touched() {
        let text = "other:\n    crash_reporting:\n        enabled: true\ninstall:\n    crash_reporting:\n        enabled: true\n";
        assert_eq!(
            with_crash_reporting_off(text).as_deref(),
            Some(
                "other:\n    crash_reporting:\n        enabled: true\ninstall:\n    crash_reporting:\n        enabled: false\n"
            )
        );
    }

    #[test]
    fn random_settings_files_keep_every_other_line_and_end_with_crash_reporting_off() {
        let mut state = 0x5EED_0401u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let lines = [
            "install:",
            "    crash_reporting:",
            "        enabled: true",
            "        enabled: false",
            "        type: \"crashpad\"",
            "    globals:",
            "        locale: \"en_US\"",
            "  crash_reporting:",
            "    enabled: true",
            "other:",
            "    crash_reporting:",
            "        enabled: true",
            "# comment",
            "",
            "patcher:",
            "    locales:",
            "    - \"en_US\"",
        ];
        for _ in 0..20_000 {
            let newline = if next() % 2 == 0 { "\r\n" } else { "\n" };
            let text: String = (0..next() % 12)
                .map(|_| format!("{}{newline}", lines[(next() % lines.len() as u64) as usize]))
                .collect();
            let Some(edited) = with_crash_reporting_off(&text) else {
                continue;
            };
            assert_eq!(with_crash_reporting_off(&edited), None, "{text:?}");
            let after: Vec<&str> = edited.lines().collect();
            let mut cursor = after.iter();
            for line in text
                .lines()
                .filter(|l| !l.trim_start().starts_with("enabled:"))
            {
                assert!(cursor.any(|l| *l == line), "{line:?} lost from {text:?}");
            }
            let install = after
                .iter()
                .position(|l| *l == "install:")
                .expect("install section");
            assert!(
                after[install + 1..]
                    .iter()
                    .take_while(|l| l.is_empty() || l.starts_with(' ') || l.starts_with('#'))
                    .any(|l| l.trim() == "enabled: false"),
                "{edited:?}"
            );
            if text.contains("\r\n") {
                assert!(
                    !edited.replace("\r\n", "").contains('\n'),
                    "line endings mixed in {edited:?}"
                );
            }
        }
    }

    #[test]
    fn the_file_lives_in_the_install_roots_config_folder() {
        let game = Path::new("D:/Riot Games/League of Legends/Game");
        assert_eq!(
            settings_path(game),
            Some(Path::new("D:/Riot Games/League of Legends/Config").join(SETTINGS_FILE))
        );
    }

    #[test]
    fn the_settings_file_is_edited_in_place_and_a_missing_one_is_skipped() {
        let root =
            std::env::temp_dir().join(format!("dekan_client_settings_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root); // ignore-ok: fixture may not exist yet
        let game = root.join("Game");
        std::fs::create_dir_all(&game).expect("game dir");
        assert!(!disable_crash_reporting(&game).expect("missing file"));
        std::fs::create_dir_all(root.join("Config")).expect("config dir");
        let path = settings_path(&game).expect("path");
        std::fs::write(&path, CLIENT).expect("write");
        assert!(disable_crash_reporting(&game).expect("edit"));
        assert!(!disable_crash_reporting(&game).expect("already off"));
        assert_eq!(
            std::fs::read_to_string(&path).expect("read"),
            CLIENT.replace("enabled: true", "enabled: false")
        );
        let _ = std::fs::remove_dir_all(&root); // ignore-ok: fixture cleanup
    }
}
