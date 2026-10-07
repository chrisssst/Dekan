use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use dekan_wad::hash::wad_path_hash;
use dekan_wad::wad::WadFile;
use tracing::{debug, warn};

use crate::error::ClassicError;

const GAME_DATA_PLUGIN: &str = "rcp-be-lol-game-data";
const GAME_DATA_ROOT: &str = "plugins/rcp-be-lol-game-data/global/default";

pub struct ClientGameData {
    archives: Vec<WadFile>,
}

#[derive(serde::Deserialize)]
struct SummaryEntry {
    id: i64,
    alias: String,
}

impl ClientGameData {
    pub fn open(client_dir: &Path) -> Result<Self, ClassicError> {
        let plugin = client_dir.join("Plugins").join(GAME_DATA_PLUGIN);
        let mut paths: Vec<PathBuf> = std::fs::read_dir(&plugin)?
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| e.eq_ignore_ascii_case("wad"))
            })
            .collect();
        paths.sort();
        let mut archives = Vec::with_capacity(paths.len());
        for path in &paths {
            match WadFile::open_toc_only(path) {
                Ok(wad) => archives.push(wad),
                Err(e) => {
                    warn!(archive = %path.display(), error = %e, "Client game data archive unreadable; skipped")
                }
            }
        }
        if archives.is_empty() {
            return Err(ClassicError::ChampionNotFound {
                alias: format!("no client game data archive in {}", plugin.display()),
            });
        }
        Ok(Self { archives })
    }

    pub fn for_game(game_dir: &Path) -> Result<Self, ClassicError> {
        let client_dir = game_dir
            .parent()
            .ok_or_else(|| ClassicError::ChampionNotFound {
                alias: format!("{} has no client folder above it", game_dir.display()),
            })?;
        Self::open(client_dir)
    }

    #[must_use]
    pub fn read(&self, relative: &str) -> Option<Vec<u8>> {
        let hash = wad_path_hash(&format!("{GAME_DATA_ROOT}/{relative}"));
        self.archives
            .iter()
            .find_map(|wad| wad.read(hash).ok().flatten())
    }

    pub fn champion_aliases(&self) -> Result<BTreeMap<u32, String>, ClassicError> {
        let bytes = self.read("v1/champion-summary.json").ok_or_else(|| {
            ClassicError::Bin("champion-summary.json is not in the client".into())
        })?;
        let entries: Vec<SummaryEntry> =
            serde_json::from_slice(&bytes).map_err(|e| ClassicError::Bin(e.to_string()))?;
        Ok(entries
            .into_iter()
            .filter_map(|entry| Some((u32::try_from(entry.id).ok()?, entry.alias)))
            .filter(|(id, alias)| *id > 0 && !alias.is_empty())
            .collect())
    }
}

#[must_use]
pub fn champion_alias(game_dir: &Path, champion_id: u32) -> Option<String> {
    let aliases = ClientGameData::for_game(game_dir).and_then(|data| data.champion_aliases());
    match aliases {
        Ok(aliases) => {
            let alias = aliases.get(&champion_id).cloned();
            debug!(
                champion_id,
                alias = ?alias,
                "Champion alias read from the installed client's game data"
            );
            alias
        }
        Err(e) => {
            warn!(
                champion_id,
                game = %game_dir.display(),
                error = %e,
                "The installed client's game data could not give the champion alias"
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client_with_summary(name: &str, summary: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("dekan_client_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root); // ignore-ok: fixture may not exist yet
        let plugin = root.join("Plugins").join(GAME_DATA_PLUGIN);
        std::fs::create_dir_all(&plugin).expect("fixture dir");
        std::fs::create_dir_all(root.join("Game")).expect("game dir");
        let hash = wad_path_hash(&format!("{GAME_DATA_ROOT}/v1/champion-summary.json"));
        let payload = summary.as_bytes();
        let mut wad = vec![0u8; 272 + 32];
        wad[0..4].copy_from_slice(b"RW\x03\x04");
        wad[268..272].copy_from_slice(&1u32.to_le_bytes());
        let offset = wad.len() as u32;
        wad[272..280].copy_from_slice(&hash.to_le_bytes());
        wad[280..284].copy_from_slice(&offset.to_le_bytes());
        wad[284..288].copy_from_slice(&(payload.len() as u32).to_le_bytes());
        wad[288..292].copy_from_slice(&(payload.len() as u32).to_le_bytes());
        wad.extend_from_slice(payload);
        std::fs::write(plugin.join("default-assets.wad"), wad).expect("write wad");
        root
    }

    #[test]
    fn test_placeholder_and_nameless_entries_are_not_champions() {
        let root = client_with_summary(
            "filters",
            r#"[{"id":0,"alias":"Zero"},{"id":1,"alias":""},{"id":2,"alias":"Olaf"}]"#,
        );
        let aliases = ClientGameData::for_game(&root.join("Game"))
            .expect("open")
            .champion_aliases()
            .expect("summary");
        assert_eq!(aliases.len(), 1);
        assert_eq!(aliases.get(&2).map(String::as_str), Some("Olaf"));
        let _ = std::fs::remove_dir_all(&root); // ignore-ok: fixture cleanup
    }

    #[test]
    fn test_a_champion_newer_than_any_table_resolves_from_the_client() {
        let root = client_with_summary(
            "summary",
            r#"[{"id":-1,"alias":"None"},{"id":804,"alias":"Yunara"},{"id":60001,"alias":"Jade_Annie"}]"#,
        );
        let game = root.join("Game");
        assert_eq!(champion_alias(&game, 804).as_deref(), Some("Yunara"));
        assert_eq!(champion_alias(&game, 999), None);
        let aliases = ClientGameData::for_game(&game)
            .expect("open")
            .champion_aliases()
            .expect("summary");
        assert!(
            !aliases.values().any(|a| a == "None"),
            "the placeholder entry is dropped"
        );
        let _ = std::fs::remove_dir_all(&root); // ignore-ok: fixture cleanup
    }

    #[test]
    fn test_no_client_folder_is_an_error_not_a_guess() {
        let missing = std::env::temp_dir().join(format!("dekan_no_client_{}", std::process::id()));
        assert!(ClientGameData::open(&missing).is_err());
        assert_eq!(champion_alias(&missing.join("Game"), 1), None);
    }
}
