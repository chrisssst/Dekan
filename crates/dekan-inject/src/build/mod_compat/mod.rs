use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use dekan_wad::hash::{mount_name, relative_path_hash, wad_path_hash};
use dekan_wad::prop::tree::{Value, parse_fields, write_fields};
use dekan_wad::prop::{FIELD_STRING, parse_prop_file, parse_prop_links, serialize_prop_file};
use dekan_wad::wad::WadFile;
use dekan_wad::writer::{WadWriter, WriterEntry, optimal_raw};
use tracing::{debug, warn};

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

    pub unreadable: Vec<String>,
}

impl ModCompat {
    #[must_use]
    pub fn is_compatible(&self) -> bool {
        self.dangling.is_empty() && self.unreadable.is_empty()
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

fn listed(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    std::fs::read_dir(dir)?
        .map(|entry| entry.map(|e| e.path()))
        .collect()
}

#[must_use]
pub fn mod_wads(mod_dir: &Path) -> Vec<PathBuf> {
    let found = listed(mod_dir).and_then(|children| {
        let Some(wad_dir) = children.into_iter().find(|p| {
            p.is_dir()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.eq_ignore_ascii_case("WAD"))
        }) else {
            return Ok(Vec::new());
        };
        listed(&wad_dir)
    });
    let mut wads: Vec<PathBuf> = match found {
        Ok(children) => children
            .into_iter()
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| ends_with_ci(n, ".wad.client"))
            })
            .collect(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => {
            warn!(mod_dir = %mod_dir.display(), error = %e, "Mod WADs could not be listed; the mod is not checked");
            Vec::new()
        }
    };
    wads.sort();
    wads
}

fn files_under(dir: &Path) -> std::io::Result<Vec<(String, PathBuf)>> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        for path in listed(&current)? {
            if path.is_dir() {
                stack.push(path);
            } else if let Ok(relative) = path.strip_prefix(dir) {
                out.push((relative.to_string_lossy().replace('\\', "/"), path));
            }
        }
    }
    out.sort();
    Ok(out)
}

#[must_use]
pub fn mod_hashes(wads: &[PathBuf]) -> HashSet<u64> {
    let mut hashes = HashSet::new();
    let (mut unreadable, mut first_error) = (0usize, None);
    for wad in wads {
        let read = if wad.is_dir() {
            files_under(wad).map(|files| {
                hashes.extend(
                    files
                        .iter()
                        .map(|(relative, _)| relative_path_hash(relative)),
                );
            })
        } else {
            WadFile::open_toc_only(wad)
                .map(|file| hashes.extend(file.toc().map(|e| e.path_hash)))
                .map_err(std::io::Error::other)
        };
        if let Err(e) = read {
            unreadable += 1;
            first_error.get_or_insert_with(|| format!("{}: {e}", wad.display()));
        }
    }
    if unreadable > 0 {
        warn!(
            unreadable,
            first_error = first_error.as_deref().unwrap_or_default(),
            "Mod WADs could not be read; links into them may be reported as dangling"
        );
    }
    hashes
}

fn starts_as_prop(file: &WadFile, hash: u64) -> Result<bool, dekan_wad::error::WadError> {
    Ok(file
        .read_prefix(hash, 4)?
        .is_some_and(|head| head == b"PROP" || head == b"PTCH"))
}

fn is_prop_bytes(bytes: &[u8]) -> bool {
    bytes.starts_with(b"PROP") || bytes.starts_with(b"PTCH")
}

fn inspect_links(
    wad: &Path,
    entry: &str,
    bytes: &[u8],
    resolvable: &dyn Fn(u64) -> bool,
    compat: &mut ModCompat,
) {
    let links = match parse_prop_links(bytes) {
        Ok(links) => links,
        Err(e) => {
            compat.unreadable.push(format!("{entry}: {e}"));
            return;
        }
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
    let unreadable_folder = |e: std::io::Error| {
        InjectError::Overlay(format!("mod folder unreadable '{}': {e}", wad.display()))
    };
    if wad.is_dir() {
        let resolvable = |hash: u64| game_hashes.contains(&hash) || mod_hashes.contains(&hash);
        for (relative, path) in files_under(wad).map_err(unreadable_folder)? {
            if !ends_with_ci(&relative, ".bin") {
                continue;
            }
            match std::fs::read(&path) {
                Ok(bytes) if is_prop_bytes(&bytes) => {
                    inspect_links(wad, &relative, &bytes, &resolvable, &mut compat);
                }
                Ok(_) => compat.entries_skipped += 1,
                Err(e) => compat.unreadable.push(format!("{relative}: {e}")),
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
        let entry = format!("{hash:016x}");
        let bytes = match starts_as_prop(&file, hash) {
            Ok(false) => {
                compat.entries_skipped += 1;
                continue;
            }
            Ok(true) => file.read(hash),
            Err(e) => Err(e),
        };
        match bytes {
            Ok(Some(bytes)) => inspect_links(wad, &entry, &bytes, &resolvable, &mut compat),
            Ok(None) => compat
                .unreadable
                .push(format!("{entry}: missing from its own table")),
            Err(e) => compat.unreadable.push(format!("{entry}: {e}")),
        }
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
