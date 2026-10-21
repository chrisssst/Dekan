use std::path::{Path, PathBuf};

use serde_json::Value;
use tracing::{info, warn};

use super::characters::wad_stamp;
use super::reuse::{Claim, generator_stamp};
use super::{CLASSIC_MOD_PREFIX, STANDARD_MOD_PREFIX, is_safe_alias, remove_if_present};

const PARTIAL_SUFFIX: &str = ".partial";
const GENERATED_AUTHOR: &str = "Dekan";

#[derive(Default)]
struct Problems {
    count: usize,
    first: Option<String>,
}

impl Problems {
    fn note(&mut self, what: String) {
        self.count += 1;
        self.first.get_or_insert(what);
    }
}

pub fn remove_stale_generated(game_dir: &Path, mods_dir: &Path, stop: impl Fn() -> bool) -> usize {
    let entries = match std::fs::read_dir(mods_dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return 0,
        Err(e) => {
            warn!(
                folder = %mods_dir.display(),
                error = %e,
                "Generated skins folder unreadable; stale skins not removed"
            );
            return 0;
        }
    };
    let mut removed = 0usize;
    let mut problems = Problems::default();
    for entry in entries {
        if stop() {
            break;
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(e) => {
                problems.note(e.to_string());
                continue;
            }
        };
        match entry.file_type() {
            Ok(kind) if kind.is_dir() => {}
            Ok(_) => continue,
            Err(e) => {
                problems.note(format!("{}: {e}", entry.path().display()));
                continue;
            }
        }
        let path = entry.path();
        if !is_stale(game_dir, &path, &mut problems) {
            continue;
        }
        let _claim = Claim::wait_for(&build_target(&path));
        if !is_stale(game_dir, &path, &mut problems) {
            continue;
        }
        match remove_if_present(&path) {
            Ok(()) => removed += 1,
            Err(e) => problems.note(format!("{}: {e}", path.display())),
        }
    }
    if problems.count > 0 {
        warn!(
            problems = problems.count,
            first = problems.first.as_deref().unwrap_or_default(),
            "Generated skins could not all be checked or removed; the next start tries again"
        );
    }
    if removed > 0 {
        info!(
            removed,
            "Removed generated skins that could no longer be reused"
        );
    }
    removed
}

fn build_target(path: &Path) -> PathBuf {
    match path.to_str().and_then(|p| p.strip_suffix(PARTIAL_SUFFIX)) {
        Some(target) => PathBuf::from(target),
        None => path.to_path_buf(),
    }
}

fn is_stale(game_dir: &Path, path: &Path, problems: &mut Problems) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let generated_name =
        name.starts_with(STANDARD_MOD_PREFIX) || name.starts_with(CLASSIC_MOD_PREFIX);
    if !generated_name || !path.exists() {
        return false;
    }
    if name.ends_with(PARTIAL_SUFFIX) {
        return true;
    }
    if name.starts_with(CLASSIC_MOD_PREFIX) {
        return written_by_dekan(path, problems);
    }
    standard_is_outdated(game_dir, path, problems)
}

fn written_by_dekan(path: &Path, problems: &mut Problems) -> bool {
    read_meta(path, "info.json", problems)
        .is_some_and(|info| info.get("Author").and_then(Value::as_str) == Some(GENERATED_AUTHOR))
}

fn standard_is_outdated(game_dir: &Path, path: &Path, problems: &mut Problems) -> bool {
    let Some(manifest) = read_meta(path, "manifest.json", problems) else {
        return false;
    };
    let field = |key: &str| manifest.get(key).and_then(Value::as_str);
    let (Some(alias), Some(generator), Some(game_wad)) =
        (field("alias"), field("generator"), field("game_wad"))
    else {
        return false;
    };
    if !is_safe_alias(alias) {
        return false;
    }
    let wad = game_dir
        .join("DATA")
        .join("FINAL")
        .join("Champions")
        .join(format!("{alias}.wad.client"));
    generator != generator_stamp() || game_wad != wad_stamp(&wad)
}

fn read_meta(path: &Path, file: &str, problems: &mut Problems) -> Option<Value> {
    let target = path.join("META").join(file);
    let bytes = match std::fs::read(&target) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            problems.note(format!("{}: {e}", target.display()));
            return None;
        }
    };
    match serde_json::from_slice::<Value>(&bytes) {
        Ok(value) if value.is_object() => Some(value),
        Ok(_) => {
            problems.note(format!("{}: not a JSON object", target.display()));
            None
        }
        Err(e) => {
            problems.note(format!("{}: {e}", target.display()));
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_meta(dir: &Path, file: &str, value: &Value) {
        std::fs::create_dir_all(dir.join("META")).expect("meta dir");
        std::fs::write(dir.join("META").join(file), value.to_string()).expect("meta file");
    }

    #[test]
    fn only_generated_skins_that_cannot_be_reused_are_removed() {
        let root =
            std::env::temp_dir().join(format!("dekan_generated_cleanup_{}", std::process::id()));
        let game = root.join("game");
        let mods = root.join("mods");
        let wad = game.join("DATA").join("FINAL").join("Champions");
        std::fs::create_dir_all(&wad).expect("game dir");
        std::fs::write(wad.join("Ahri.wad.client"), b"wad").expect("wad file");
        let current = wad_stamp(&wad.join("Ahri.wad.client"));

        let manifest = |generator: &str, game_wad: &str| serde_json::json!({ "alias": "Ahri", "generator": generator, "game_wad": game_wad });
        write_meta(
            &mods.join("std_ahri_1"),
            "manifest.json",
            &manifest(generator_stamp(), &current),
        );
        write_meta(
            &mods.join("std_ahri_2"),
            "manifest.json",
            &manifest(generator_stamp(), "0:0"),
        );
        write_meta(
            &mods.join("std_ahri_3"),
            "manifest.json",
            &manifest("0.1+old", &current),
        );
        write_meta(
            &mods.join("classic_ahri_1"),
            "info.json",
            &serde_json::json!({ "Author": "Dekan" }),
        );
        write_meta(
            &mods.join("classic_mine"),
            "info.json",
            &serde_json::json!({ "Author": "Someone" }),
        );
        std::fs::create_dir_all(mods.join("std_mine")).expect("custom mod");
        std::fs::create_dir_all(mods.join("std_ahri_4.partial")).expect("partial");

        let removed = remove_stale_generated(&game, &mods, || false);

        let left = |name: &str| mods.join(name).exists();
        assert_eq!(removed, 4);
        assert!(left("std_ahri_1"), "reusable skin kept");
        assert!(!left("std_ahri_2"), "another game archive");
        assert!(!left("std_ahri_3"), "another Dekan build");
        assert!(
            !left("classic_ahri_1"),
            "classic skins are always regenerated"
        );
        assert!(
            left("classic_mine") && left("std_mine"),
            "folders Dekan did not write are kept"
        );
        assert!(
            !left("std_ahri_4.partial"),
            "an interrupted build is removed"
        );
        let _ = std::fs::remove_dir_all(&root); // ignore-ok: cleanup test directory
    }
}
