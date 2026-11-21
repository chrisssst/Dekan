use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use dekan_wad::hash::wad_path_hash;
use dekan_wad::prop::tree::{self, Value};
use dekan_wad::prop::{FIELD_STRING, parse_prop_file};
use dekan_wad::wad::WadFile;
use sha2::{Digest, Sha256};

const SKIN_DATA_CLASS: u32 = 0x9b67_e9f6;

fn h(name: &str) -> u32 {
    let mut hash: u32 = 0x811c_9dc5;
    for b in name.bytes() {
        hash = (hash ^ u32::from(b.to_ascii_lowercase())).wrapping_mul(0x0100_0193);
    }
    hash
}

fn raw_text(value: &Value) -> Option<String> {
    match value {
        Value::Raw {
            kind: FIELD_STRING,
            bytes,
        } => bytes
            .get(2..)
            .map(|text| String::from_utf8_lossy(text).into_owned()),
        _ => None,
    }
}

fn sha256_half(bytes: &[u8]) -> [u8; 16] {
    let digest = Sha256::digest(bytes);
    let mut out = [0u8; 16];
    out.copy_from_slice(&digest[..16]);
    out
}

fn sha256_half_u32_size(bytes: &[u8], size: usize) -> [u8; 16] {
    let mut hasher = Sha256::new();
    let len_le = u32::try_from(size).unwrap_or(u32::MAX).to_le_bytes();
    hasher.update(len_le);
    if bytes.len() > 4 {
        hasher.update(&bytes[4..size]);
    }
    let digest = hasher.finalize();
    let mut out = [0u8; 16];
    out.copy_from_slice(&digest[..16]);
    out
}

fn contains_skin_ref(value: &str, needles: &[String]) -> bool {
    let lower = value.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    needles.iter().any(|needle| {
        let mut from = 0;
        while let Some(pos) = lower[from..].find(needle) {
            let after = from + pos + needle.len();
            let digits_end = bytes[after..]
                .iter()
                .take_while(|b| b.is_ascii_digit())
                .count();
            if digits_end > 0 && bytes[after..after + digits_end].iter().any(|&b| b != b'0') {
                return true;
            }
            from = after;
        }
        false
    })
}

fn skin_needles(character: &str, wad_stem: &str) -> Vec<String> {
    let mut out = Vec::new();
    for raw in [character, wad_stem] {
        let low = raw.to_ascii_lowercase();
        let plain = low.strip_prefix("jade_").unwrap_or(&low);
        for item in [low.as_str(), plain] {
            if !item.is_empty() {
                let needle = format!("{item}skin");
                if !out.contains(&needle) {
                    out.push(needle);
                }
            }
        }
    }
    out
}

fn collect_mod_files(root: &Path, base: &Path, out: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_mod_files(&path, base, out);
        } else if let Ok(rel) = path.strip_prefix(base) {
            let norm = rel
                .to_string_lossy()
                .replace('\\', "/")
                .to_ascii_lowercase();
            out.push((norm, path));
        }
    }
}

fn check_skin0_bin(
    rel: &str,
    bytes: &[u8],
    character: &str,
    wad_stem: &str,
    failures: &mut Vec<String>,
) {
    let parsed = match parse_prop_file(bytes) {
        Ok(p) => p,
        Err(e) => {
            failures.push(format!(
                "[verify_ah:bin] {rel}: parse_prop_file failed: {e}"
            ));
            return;
        }
    };
    let Some(entry) = parsed
        .entries
        .iter()
        .find(|e| e.class_hash == SKIN_DATA_CLASS)
    else {
        return;
    };
    let fields = match tree::parse_fields(&entry.body) {
        Ok(f) => f,
        Err(e) => {
            failures.push(format!("[verify_ah:bin] {rel}: parse_fields failed: {e}"));
            return;
        }
    };
    let needles = skin_needles(character, wad_stem);
    if let Some(name) = tree::field(&fields, h("championSkinName")).and_then(raw_text) {
        if contains_skin_ref(&name, &needles) {
            failures.push(format!("[verify_ah:bin] {rel}: championSkinName={name}"));
        }
    }
    let Some(audio) = tree::field(&fields, h("skinAudioProperties")).and_then(Value::fields) else {
        return;
    };
    if let Some(tags) = tree::field(audio, h("tagEventList")).and_then(Value::items) {
        for tag in tags.iter().filter_map(raw_text) {
            if contains_skin_ref(&tag, &needles) {
                failures.push(format!("[verify_ah:bin] {rel}: tagEventList={tag}"));
            }
        }
    }
    if let Some(units) = tree::field(audio, h("bankUnits")).and_then(Value::items) {
        for unit in units {
            let Some(uf) = unit.fields() else {
                continue;
            };
            if let Some(uname) = tree::field(uf, h("name")).and_then(raw_text) {
                if contains_skin_ref(&uname, &needles) {
                    failures.push(format!("[verify_ah:bin] {rel}: bankUnits.name={uname}"));
                }
            }
            if let Some(evs) = tree::field(uf, h("events")).and_then(Value::items) {
                for ev in evs.iter().filter_map(raw_text) {
                    if contains_skin_ref(&ev, &needles) {
                        failures.push(format!("[verify_ah:bin] {rel}: bankUnits.events={ev}"));
                        break;
                    }
                }
            }
        }
    }
}

struct OtherMeshHashes {
    skin0_skl: Option<[u8; 16]>,
    skin0_skn: Option<[u8; 16]>,
    plain: HashSet<[u8; 16]>,
    u32_size: HashSet<[u8; 16]>,
    lengths: BTreeSet<usize>,
}

fn read_mesh_paths(bytes: &[u8]) -> (Option<String>, Option<String>) {
    let Ok(parsed) = parse_prop_file(bytes) else {
        return (None, None);
    };
    let Some(entry) = parsed
        .entries
        .iter()
        .find(|e| e.class_hash == SKIN_DATA_CLASS)
    else {
        return (None, None);
    };
    let Ok(fields) = tree::parse_fields(&entry.body) else {
        return (None, None);
    };
    let mesh = tree::field(&fields, h("skinMeshProperties")).and_then(Value::fields);
    let skl = mesh.and_then(|m| tree::field(m, h("skeleton")).and_then(raw_text));
    let skn = mesh.and_then(|m| tree::field(m, h("simpleSkin")).and_then(raw_text));
    (skl, skn)
}

fn collect_other_meshes(original: &WadFile, character: &str) -> OtherMeshHashes {
    let mut out = OtherMeshHashes {
        skin0_skl: None,
        skin0_skn: None,
        plain: HashSet::new(),
        u32_size: HashSet::new(),
        lengths: BTreeSet::new(),
    };
    if let Ok(Some(skin0)) = original.read(wad_path_hash(&format!(
        "data/characters/{character}/skins/skin0.bin"
    ))) {
        let (skl, skn) = read_mesh_paths(&skin0);
        if let Some(p) = skl.and_then(|p| {
            original
                .read(wad_path_hash(&p.to_ascii_lowercase()))
                .ok()
                .flatten()
        }) {
            out.skin0_skl = Some(sha256_half(&p));
        }
        if let Some(p) = skn.and_then(|p| {
            original
                .read(wad_path_hash(&p.to_ascii_lowercase()))
                .ok()
                .flatten()
        }) {
            out.skin0_skn = Some(sha256_half(&p));
        }
    }
    for n in 1..=150u32 {
        let bin_path = format!("data/characters/{character}/skins/skin{n}.bin");
        let Ok(Some(bin)) = original.read(wad_path_hash(&bin_path)) else {
            continue;
        };
        let (skl, skn) = read_mesh_paths(&bin);
        for path_opt in [skl, skn] {
            let Some(path) = path_opt else {
                continue;
            };
            let Ok(Some(bytes)) = original.read(wad_path_hash(&path.to_ascii_lowercase())) else {
                continue;
            };
            let h_plain = sha256_half(&bytes);
            if Some(h_plain) == out.skin0_skl || Some(h_plain) == out.skin0_skn {
                continue;
            }
            out.lengths.insert(bytes.len());
            out.plain.insert(h_plain);
            if bytes.len() >= 4 {
                out.u32_size
                    .insert(sha256_half_u32_size(&bytes, bytes.len()));
            }
        }
    }
    out
}

fn check_mesh_prefixes(
    rel: &str,
    bytes: &[u8],
    other: &OtherMeshHashes,
    failures: &mut Vec<String>,
) {
    let trimmed = bytes.iter().rposition(|&b| b != 0).map_or(0, |idx| idx + 1);
    for &len in &other.lengths {
        if len > bytes.len() {
            continue;
        }
        if other.plain.contains(&sha256_half(&bytes[..len])) {
            failures.push(format!(
                "[verify_ah:mesh] {rel}: prefix[..{len}] matches another skin's mesh (len={})",
                bytes.len()
            ));
            return;
        }
        if len >= trimmed && len >= 4 && other.u32_size.contains(&sha256_half_u32_size(bytes, len))
        {
            failures.push(format!(
                "[verify_ah:mesh] {rel}: u32_size({len}) prefix matches another skin's mesh (trimmed={trimmed}, len={})",
                bytes.len()
            ));
            return;
        }
    }
}

pub(super) fn check_mod_against_ah(
    mod_wad_dir: &Path,
    overlay_wad: &WadFile,
    original_wad: &WadFile,
    wad_stem: &str,
    failures: &mut Vec<String>,
) {
    let mut mod_files = Vec::new();
    collect_mod_files(mod_wad_dir, mod_wad_dir, &mut mod_files);
    let mut mesh_cache: HashMap<String, OtherMeshHashes> = HashMap::new();
    for (rel, path) in mod_files {
        let hash = wad_path_hash(&rel);
        let Ok(file_bytes) = std::fs::read(&path) else {
            failures.push(format!("[verify_ah] unreadable mod file: {rel}"));
            continue;
        };
        let Ok(Some(ov_bytes)) = overlay_wad.read(hash) else {
            failures.push(format!("[verify_ah] {rel} missing in overlay WAD"));
            continue;
        };
        if ov_bytes != file_bytes {
            failures.push(format!("[verify_ah] {rel} overlay bytes mismatch mod file"));
            continue;
        }
        if rel.starts_with("data/characters/") && rel.ends_with("/skins/skin0.bin") {
            let parts: Vec<&str> = rel.split('/').collect();
            if parts.len() == 5 {
                let ch = parts[2];
                check_skin0_bin(&rel, &ov_bytes, ch, wad_stem, failures);
                let (skl, skn) = read_mesh_paths(&ov_bytes);
                let other = mesh_cache
                    .entry(ch.to_owned())
                    .or_insert_with(|| collect_other_meshes(original_wad, ch));
                for mesh_path in [skl, skn].into_iter().flatten() {
                    let norm = mesh_path.to_ascii_lowercase();
                    if let Ok(Some(mesh_bytes)) = overlay_wad.read(wad_path_hash(&norm)) {
                        check_mesh_prefixes(&norm, &mesh_bytes, other, failures);
                    }
                }
            }
        } else if rel.starts_with("assets/characters/")
            && (rel.ends_with(".skl") || rel.ends_with(".skn"))
            && rel.contains("/skins/base/")
        {
            let parts: Vec<&str> = rel.split('/').collect();
            if parts.len() >= 6 {
                let ch = parts[2];
                let other = mesh_cache
                    .entry(ch.to_owned())
                    .or_insert_with(|| collect_other_meshes(original_wad, ch));
                check_mesh_prefixes(&rel, &ov_bytes, other, failures);
            }
        }
    }
}
