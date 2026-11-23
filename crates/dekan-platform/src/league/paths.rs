use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tracing::debug;

use crate::error::PlatformError;

pub fn install_dir() -> Result<PathBuf, PlatformError> {
    if let Ok(base) = std::env::var("PROGRAMFILES") {
        return Ok(PathBuf::from(base).join("Dekan"));
    }
    #[cfg(target_os = "macos")]
    {
        return Ok(PathBuf::from("/Applications/Dekan.app/Contents/MacOS"));
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err(PlatformError::Path("PROGRAMFILES not set".into()))
    }
}

pub fn data_dir() -> Result<PathBuf, PlatformError> {
    Ok(crate::user_profile::local_app_data()?.join("Dekan"))
}

pub fn logs_dir() -> Result<PathBuf, PlatformError> {
    Ok(data_dir()?.join("logs"))
}

pub const STAR_GATE_FILE: &str = "star_gate.bin";

pub fn state_dir() -> Result<PathBuf, PlatformError> {
    Ok(data_dir()?.join("state"))
}

#[must_use]
pub fn tools_dir_candidates(data_dir: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(override_dir) = std::env::var("DEKAN_TOOLS_DIR") {
        let p = PathBuf::from(override_dir);
        if !p.as_os_str().is_empty() {
            candidates.push(p);
        }
    }
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
    {
        let beside = dir.join("tools");
        if !candidates.contains(&beside) {
            candidates.push(beside);
        }
    }
    if let Ok(install) = install_dir() {
        let p = install.join("tools");
        if !candidates.contains(&p) {
            candidates.push(p);
        }
    }
    candidates.push(data_dir.join("tools"));
    candidates
}

#[must_use]
pub fn riot_league_installs() -> Vec<PathBuf> {
    riot_installs(&mut Vec::new())
}

fn riot_metadata_dir(notes: &mut Vec<String>) -> Option<PathBuf> {
    if let Ok(program_data) = std::env::var("ProgramData") {
        return Some(
            PathBuf::from(program_data)
                .join("Riot Games")
                .join("Metadata"),
        );
    }
    #[cfg(target_os = "macos")]
    {
        let shared = PathBuf::from("/Users/Shared/Riot Games/Metadata");
        if shared.exists() {
            return Some(shared);
        }
    }
    notes.push("ProgramData is not set".into());
    None
}

#[cfg_attr(not(target_os = "macos"), allow(unused_variables, clippy::ptr_arg))]
fn append_macos_default_installs(installs: &mut Vec<PathBuf>) {
    #[cfg(target_os = "macos")]
    {
        let mut defaults = vec![
            PathBuf::from("/Applications/League of Legends.app/Contents/LoL"),
            PathBuf::from("/Applications/League of Legends.app"),
        ];
        if let Some(home) = std::env::home_dir() {
            defaults.push(home.join("Applications/League of Legends.app/Contents/LoL"));
            defaults.push(home.join("Applications/League of Legends.app"));
        }
        for candidate in defaults {
            if candidate.exists() && !installs.contains(&candidate) {
                installs.push(candidate);
            }
        }
    }
}

fn riot_installs(notes: &mut Vec<String>) -> Vec<PathBuf> {
    let mut installs = Vec::new();
    let Some(metadata) = riot_metadata_dir(notes) else {
        append_macos_default_installs(&mut installs);
        return installs;
    };
    let entries = match std::fs::read_dir(&metadata) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            notes.push(format!("no Riot metadata folder at {}", metadata.display()));
            append_macos_default_installs(&mut installs);
            return installs;
        }
        Err(e) => {
            notes.push(format!(
                "Riot metadata folder {} unreadable: {e}",
                metadata.display()
            ));
            append_macos_default_installs(&mut installs);
            return installs;
        }
    };
    let mut patchlines: Vec<String> = Vec::new();
    for entry in entries {
        match entry {
            Ok(entry) if entry.path().is_dir() => {
                if let Some(name) = entry.file_name().to_str().map(str::to_owned) {
                    patchlines.push(name);
                }
            }
            Ok(_) => {}
            Err(e) => notes.push(format!("Riot metadata entry unreadable: {e}")),
        }
    }
    patchlines.retain(|name| {
        name.strip_prefix("league_of_legends.")
            .is_some_and(|rest| !rest.is_empty() && !rest.contains('.'))
    });

    patchlines.sort_by_key(|name| (name != "league_of_legends.live", name.clone()));

    for name in &patchlines {
        let settings = metadata
            .join(name)
            .join(format!("{name}.product_settings.yaml"));
        match std::fs::read_to_string(&settings) {
            Ok(text) => match install_path_from_settings(&text) {
                Some(path) => installs.push(path),
                None => notes.push(format!("{name} registers no install path")),
            },
            Err(e) => notes.push(format!("{} unreadable: {e}", settings.display())),
        }
    }
    append_macos_default_installs(&mut installs);
    installs
}

fn install_path_from_settings(text: &str) -> Option<PathBuf> {
    text.lines().find_map(|line| {
        let value = line.strip_prefix("product_install_full_path:")?.trim();
        let value = value.trim_matches('"').trim_matches('\'');
        (!value.is_empty()).then(|| PathBuf::from(value))
    })
}

#[must_use]
pub fn discover_game_dir() -> Option<PathBuf> {
    discover_game_dir_with_source().map(|(dir, _)| dir)
}

static LAST_MISS: Mutex<String> = Mutex::new(String::new());

fn remember_miss(summary: &str) {
    let mut last = LAST_MISS.lock().unwrap_or_else(|e| e.into_inner());
    if *last != summary {
        debug!(reasons = summary, "League install not found");
        *last = summary.to_owned();
    }
}

fn clear_miss() {
    LAST_MISS.lock().unwrap_or_else(|e| e.into_inner()).clear();
}

#[must_use]
pub fn discover_game_dir_with_source() -> Option<(PathBuf, &'static str)> {
    let mut notes = Vec::new();
    for exe in crate::game_version::GAME_EXES
        .into_iter()
        .chain(crate::game_version::CLIENT_EXES)
    {
        match crate::process::ProcessFinder::find_process_path(exe) {
            Ok(Some(path)) => match path.parent().and_then(normalize_game_dir) {
                Some(valid) => {
                    clear_miss();
                    return Some((valid, exe));
                }
                None => notes.push(format!(
                    "{exe} runs from {}, which is not a game install",
                    path.display()
                )),
            },
            Ok(None) => {}
            Err(e) => notes.push(format!("{exe} lookup failed: {e}")),
        }
    }
    for install in riot_installs(&mut notes) {
        match normalize_game_dir(&install) {
            Some(dir) => {
                clear_miss();
                return Some((dir, "riot_client_installs"));
            }
            None => notes.push(format!(
                "registered install {} is not a game install",
                install.display()
            )),
        }
    }
    remember_miss(&notes.join("; "));
    None
}

pub fn is_valid_game_dir(path: &Path) -> bool {
    crate::game_version::game_exe(path).is_file() && path.join("DATA").is_dir()
}

pub fn normalize_game_dir(path: &Path) -> Option<PathBuf> {
    if is_valid_game_dir(path) {
        return Some(path.to_path_buf());
    }
    for candidate in [
        path.join("Game"),
        path.with_file_name("Game"),
        path.join("Contents/LoL/Game"),
        path.join("LoL/Game"),
    ] {
        if is_valid_game_dir(&candidate) {
            return Some(candidate);
        }
    }
    for ancestor in path.ancestors().skip(1).take(7) {
        if is_valid_game_dir(ancestor) {
            return Some(ancestor.to_path_buf());
        }
        for sub in ["Game", "Contents/LoL/Game"] {
            let candidate = ancestor.join(sub);
            if is_valid_game_dir(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_the_install_path_is_read_from_the_riot_settings_file() {
        let settings = "auto_patching_enabled_by_player: false\n\
                        product_install_full_path: \"D:/Riot Games/League of Legends\"\n\
                        product_install_root: \"D:/Riot Games\"\n";
        assert_eq!(
            install_path_from_settings(settings),
            Some(PathBuf::from("D:/Riot Games/League of Legends"))
        );
        assert_eq!(
            install_path_from_settings("product_install_root: \"D:/x\"\n"),
            None
        );
        assert_eq!(
            install_path_from_settings("product_install_full_path: \"\"\n"),
            None
        );
    }

    #[test]
    fn test_tools_candidates_all_belong_to_dekan_and_end_with_the_data_folder() {
        let data = PathBuf::from(r"X:\Data\Dekan");
        let candidates = tools_dir_candidates(&data);
        assert_eq!(candidates.last(), Some(&data.join("tools")));
        assert!(candidates.iter().all(|c| c.ends_with("tools")));
        if let Ok(install) = install_dir() {
            assert!(candidates.contains(&install.join("tools")));
        }
    }

    #[test]
    fn test_a_wegame_install_with_sibling_folders_and_the_tm_executable_is_found() {
        let temp = std::env::temp_dir().join(format!("dekan_test_wegame_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp); // ignore-ok: fixture may not exist yet
        let client = temp.join("LeagueClient");
        let game = temp.join("Game");
        std::fs::create_dir_all(&client).expect("client dir");
        std::fs::create_dir_all(game.join("DATA")).expect("game DATA");
        std::fs::write(game.join("League of Legends (TM) Client.exe"), b"exe").expect("game exe");

        assert!(is_valid_game_dir(&game));
        assert_eq!(normalize_game_dir(&client), Some(game.clone()));
        assert_eq!(
            crate::game_version::game_exe(&game),
            game.join("League of Legends (TM) Client.exe")
        );

        let _ = std::fs::remove_dir_all(&temp); // ignore-ok: cleanup fixture after test
    }

    #[test]
    fn test_is_valid_game_dir_distinguishes_game_from_client_root() {
        let temp = std::env::temp_dir().join(format!("dekan_test_game_dir_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp); // ignore-ok: cleanup fixture before test

        let client_root = temp.join("League of Legends");
        std::fs::create_dir_all(client_root.join("DATA")).expect("create client DATA");
        std::fs::write(client_root.join("LeagueClientUx.exe"), b"mock client")
            .expect("write client");

        let game_dir = client_root.join("Game");
        std::fs::create_dir_all(game_dir.join("DATA")).expect("create game DATA");
        std::fs::write(game_dir.join("League of Legends.exe"), b"mock game exe")
            .expect("write game exe");

        assert!(
            !is_valid_game_dir(&client_root),
            "client root must not be accepted as game dir"
        );
        assert!(
            is_valid_game_dir(&game_dir),
            "game dir with exe and DATA must be valid"
        );

        assert_eq!(normalize_game_dir(&client_root), Some(game_dir.clone()));

        assert_eq!(normalize_game_dir(&game_dir), Some(game_dir));

        let mac_app = temp.join("League of Legends.app");
        let mac_game = mac_app.join("Contents/LoL/Game");
        let mac_exe_dir = mac_game.join("LeagueofLegends.app/Contents/MacOS");
        let mac_client_dir = mac_app.join(
            "Contents/LoL/LeagueClient.app/Contents/Helpers/LeagueClientUx.app/Contents/MacOS",
        );
        std::fs::create_dir_all(mac_game.join("DATA")).expect("mac DATA");
        std::fs::create_dir_all(&mac_exe_dir).expect("mac exe dir");
        std::fs::create_dir_all(&mac_client_dir).expect("mac client dir");
        std::fs::write(mac_exe_dir.join("LeagueofLegends"), b"macho").expect("mac exe");
        assert!(is_valid_game_dir(&mac_game));
        assert_eq!(normalize_game_dir(&mac_app), Some(mac_game.clone()));
        assert_eq!(normalize_game_dir(&mac_exe_dir), Some(mac_game.clone()));
        assert_eq!(normalize_game_dir(&mac_client_dir), Some(mac_game));

        let _ = std::fs::remove_dir_all(&temp); // ignore-ok: cleanup fixture after test
    }

    #[test]
    fn test_the_discovered_install_names_where_it_was_found_and_is_a_real_game_folder() {
        let found = discover_game_dir_with_source();
        assert_eq!(
            found.as_ref().map(|(dir, _)| dir.clone()),
            discover_game_dir()
        );
        let Some((dir, found_by)) = found else {
            return;
        };
        assert!(is_valid_game_dir(&dir), "{}", dir.display());
        let from_process = crate::game_version::GAME_EXES
            .into_iter()
            .chain(crate::game_version::CLIENT_EXES)
            .any(|exe| exe == found_by);
        if !from_process {
            assert_eq!(found_by, "riot_client_installs");
            assert!(
                riot_league_installs()
                    .iter()
                    .any(|install| normalize_game_dir(install).as_ref() == Some(&dir))
            );
        }
    }
}
