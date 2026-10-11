use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use dekan_wad::hash::{mount_name, relative_path_hash, wad_path_hash};
use dekan_wad::prop::tree::{Value, parse_fields, write_fields};
use dekan_wad::prop::{FIELD_STRING, parse_prop_file, parse_prop_links, serialize_prop_file};
use dekan_wad::wad::WadFile;
use dekan_wad::writer::{WadWriter, WriterEntry, optimal_raw};
use tracing::debug;

use crate::error::InjectError;
use crate::overlay_builder::GameWad;

const SHARED_SKINS_MARKER: &str = "_multi_skins_";

const LEGACY_SHARED_SKINS_MARKER: &str = "_skins_";

const SKIN_SLOTS_SCANNED: u32 = 256;

const FORMAT_PREFIX: usize = 16;

const SKL_FORMAT_TOKEN: u32 = 0x22FD_4FC3;

const FORMAT_REFERENCE_MAX_ENTRIES: usize = 12_000;

const ASSET_TWINS: [(&str, &str); 4] = [
    ("dds", "tex"),
    ("tex", "dds"),
    ("sco", "scb"),
    ("scb", "sco"),
];

const ASSET_EXTENSIONS: [&str; 9] = [
    "tex", "dds", "skn", "skl", "anm", "scb", "sco", "bnk", "wpk",
];

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModCompat {
    pub dangling: Vec<String>,

    pub props_checked: usize,

    pub entries_skipped: usize,
}

impl ModCompat {
    #[must_use]
    pub fn is_compatible(&self) -> bool {
        self.dangling.is_empty()
    }
}

fn is_bin_link(link: &str) -> bool {
    link.rsplit('.')
        .next()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("bin"))
}

fn ends_with_ci(name: &str, suffix: &str) -> bool {
    name.len() >= suffix.len()
        && name.is_char_boundary(name.len() - suffix.len())
        && name[name.len() - suffix.len()..].eq_ignore_ascii_case(suffix)
}

#[must_use]
pub fn mod_wads(mod_dir: &Path) -> Vec<PathBuf> {
    let Some(wad_dir) = std::fs::read_dir(mod_dir).ok().and_then(|entries| {
        entries.flatten().map(|e| e.path()).find(|p| {
            p.is_dir()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.eq_ignore_ascii_case("WAD"))
        })
    }) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&wad_dir) else {
        return Vec::new();
    };
    let mut wads: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| ends_with_ci(n, ".wad.client"))
        })
        .collect();
    wads.sort();
    wads
}

fn files_under(dir: &Path) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        for path in entries.flatten().map(|e| e.path()) {
            if path.is_dir() {
                stack.push(path);
            } else if let Ok(relative) = path.strip_prefix(dir) {
                out.push((relative.to_string_lossy().replace('\\', "/"), path));
            }
        }
    }
    out.sort();
    out
}

#[must_use]
pub fn mod_hashes(wads: &[PathBuf]) -> HashSet<u64> {
    let mut hashes = HashSet::new();
    for wad in wads {
        if wad.is_dir() {
            hashes.extend(
                files_under(wad)
                    .iter()
                    .map(|(relative, _)| relative_path_hash(relative)),
            );
        } else if let Ok(file) = WadFile::open_toc_only(wad) {
            hashes.extend(file.toc().map(|e| e.path_hash));
        }
    }
    hashes
}

fn starts_as_prop(file: &WadFile, hash: u64) -> bool {
    file.read_prefix(hash, 4)
        .ok()
        .flatten()
        .is_some_and(|head| head == b"PROP" || head == b"PTCH")
}

fn inspect_links(
    wad: &Path,
    bytes: Option<Vec<u8>>,
    resolvable: &dyn Fn(u64) -> bool,
    compat: &mut ModCompat,
) {
    let Some(links) = bytes.and_then(|b| parse_prop_links(&b).ok()) else {
        compat.entries_skipped += 1;
        return;
    };
    compat.props_checked += 1;
    for link in links {
        if is_bin_link(&link) && !resolvable(wad_path_hash(&link)) {
            debug!(
                wad = %wad.display(),
                link = %link,
                "Dangling PROP link: the target .bin is in neither the game nor the mod"
            );
            compat.dangling.push(link);
        }
    }
}

pub fn check(
    wad: &Path,
    game_hashes: &HashSet<u64>,
    mod_hashes: &HashSet<u64>,
) -> Result<ModCompat, InjectError> {
    let mut compat = ModCompat::default();
    if wad.is_dir() {
        let resolvable = |hash: u64| game_hashes.contains(&hash) || mod_hashes.contains(&hash);
        for (relative, path) in files_under(wad) {
            if ends_with_ci(&relative, ".bin") {
                inspect_links(wad, std::fs::read(&path).ok(), &resolvable, &mut compat);
            }
        }
        return Ok(compat);
    }

    let file = WadFile::open(wad).map_err(|e| {
        InjectError::Overlay(format!("mod WAD unreadable '{}': {e}", wad.display()))
    })?;
    let hashes: Vec<u64> = file.toc().map(|e| e.path_hash).collect();
    let own: HashSet<u64> = hashes.iter().copied().collect();
    let resolvable = |hash: u64| {
        game_hashes.contains(&hash) || mod_hashes.contains(&hash) || own.contains(&hash)
    };
    for hash in hashes {
        if !starts_as_prop(&file, hash) {
            compat.entries_skipped += 1;
            continue;
        }
        inspect_links(
            wad,
            file.read(hash).ok().flatten(),
            &resolvable,
            &mut compat,
        );
    }
    Ok(compat)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relink {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Repair {
    pub relinks: Vec<Relink>,
    pub files: Vec<PathBuf>,
}

struct SharedBin {
    folder: String,
    owner: String,
    root: bool,
    slots: BTreeSet<u32>,
}

fn shared_bin(link: &str) -> Option<SharedBin> {
    let lower = link.to_ascii_lowercase();
    let (folder, file) = lower.rsplit_once('/')?;
    let file = file.strip_suffix(".bin")?;
    let (folder, owner, rest) = match file.split_once(SHARED_SKINS_MARKER) {
        Some((owner, rest)) => (folder.to_owned(), owner, rest),
        None if folder == "data" => {
            let (owner, rest) = file.split_once(LEGACY_SHARED_SKINS_MARKER)?;
            (format!("data/characters/{owner}"), owner, rest)
        }
        None => return None,
    };
    let slots: BTreeSet<u32> = rest
        .split('_')
        .filter_map(|token| token.strip_prefix("skin")?.parse().ok())
        .collect();
    if slots.is_empty() {
        return None;
    }
    Some(SharedBin {
        folder,
        owner: owner.to_owned(),
        root: rest.starts_with("root_"),
        slots,
    })
}

fn successor<'a>(
    old: &SharedBin,
    known: &BTreeSet<u32>,
    pool: &'a BTreeSet<String>,
) -> Option<&'a String> {
    let same_group: Vec<(&String, BTreeSet<u32>)> = pool
        .iter()
        .filter_map(|name| {
            let new = shared_bin(name)?;
            (new.folder == old.folder
                && new.owner == old.owner
                && new.root == old.root
                && new.slots.is_superset(&old.slots)
                && new.slots.difference(&old.slots).all(|s| !known.contains(s)))
            .then_some((name, new.slots))
        })
        .collect();
    let single = |found: Vec<&'a String>| (found.len() == 1).then(|| found[0]);
    single(
        same_group
            .iter()
            .filter(|(_, slots)| *slots == old.slots)
            .map(|(name, _)| *name)
            .collect(),
    )
    .or_else(|| single(same_group.iter().map(|(name, _)| *name).collect()))
}

mod assets;
mod repair;

use assets::*;
pub use assets::{AssetFormat, asset_format};
pub use repair::Repairer;

#[must_use]
pub fn game_hash_set(
    game: &std::collections::BTreeMap<String, crate::overlay_builder::GameWad>,
) -> HashSet<u64> {
    game.values()
        .flat_map(|wad| wad.names.iter().copied())
        .collect()
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod assets_tests;

#[cfg(test)]
mod edge_tests;
