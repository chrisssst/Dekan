use super::*;

pub(super) fn maps_holding(maps: &MapIndex, hash: u64) -> Vec<&str> {
    maps.iter()
        .filter(|(_, hashes)| hashes.contains(&hash))
        .map(|(name, _)| name.as_str())
        .collect()
}

pub(super) fn skin_bin(character: &str, skin: u32) -> String {
    format!("data/characters/{character}/skins/skin{skin}.bin")
}

pub(super) fn keeps_source_links(
    champion: &StandardChampion,
    main: &str,
    skin: u32,
    character_dir: &Path,
) -> bool {
    let links = |bytes: Vec<u8>| {
        dekan_wad::prop::parse_prop_file(&bytes)
            .ok()
            .map(|bin| bin.links)
    };
    let source = champion
        .read_skin_bin(main, skin)
        .ok()
        .flatten()
        .and_then(links);
    let generated = std::fs::read(character_dir.join("skins").join("skin0.bin"))
        .ok()
        .and_then(links);
    match (source, generated) {
        (Some(source), Some(generated)) => source.iter().all(|link| generated.contains(link)),
        _ => false,
    }
}

pub(super) fn source_keys(character: &str, links: &[String]) -> HashSet<u32> {
    let source = links.first().and_then(|link| {
        let lower = link.to_ascii_lowercase();
        let number = lower
            .strip_prefix(&format!(
                "data/characters/{}/skins/skin",
                character.to_ascii_lowercase()
            ))?
            .strip_suffix(".bin")?;
        number.parse::<u32>().ok()
    });
    source
        .into_iter()
        .flat_map(|n| {
            let object = format!("Characters/{character}/Skins/Skin{n}");
            [
                dekan_wad::hash::prop_key_hash(&object),
                dekan_wad::hash::prop_key_hash(&format!("{object}/Resources")),
            ]
        })
        .collect()
}

pub(super) fn check_mod_metadata(mod_root: &Path, finding: &mut SkinFinding) {
    for rel in ["META/info.json", "META/manifest.json"] {
        let path = mod_root.join(rel);
        let Ok(bytes) = std::fs::read(&path) else {
            finding
                .stale_references
                .push(format!("missing metadata file '{rel}'"));
            continue;
        };
        if serde_json::from_slice::<serde_json::Value>(&bytes).is_err() {
            finding
                .stale_references
                .push(format!("invalid JSON in metadata file '{rel}'"));
        }
    }
}

pub(super) fn check_generated_bins(
    champion: &StandardChampion,
    maps: &MapIndex,
    characters: &Path,
    written: &HashSet<String>,
    finding: &mut SkinFinding,
) {
    let Ok(entries) = std::fs::read_dir(characters) else {
        return;
    };
    for entry in entries.flatten() {
        let character = entry.file_name().to_string_lossy().into_owned();
        let Ok(bytes) = std::fs::read(entry.path().join("skins").join("skin0.bin")) else {
            continue;
        };
        let Ok(bin) = dekan_wad::prop::parse_prop_file(&bytes) else {
            finding
                .stale_references
                .push(format!("{character}: generated bin unreadable"));
            continue;
        };
        for prop_entry in &bin.entries {
            if dekan_wad::prop::tree::parse_fields(&prop_entry.body).is_err() {
                finding.stale_references.push(format!(
                    "{character}: entry {:08x} failed field parse",
                    prop_entry.key_hash
                ));
            }
        }
        let stale = source_keys(&character, &bin.links);
        let stuck = bin
            .entries
            .iter()
            .filter_map(|e| dekan_wad::prop::reference_values(&e.body).ok())
            .flatten()
            .filter(|value| stale.contains(value))
            .count();
        if stuck > 0 {
            finding
                .stale_references
                .push(format!("{character}: {stuck} reference(s) to a source key"));
        }
        for link in &bin.links {
            let hash = wad_path_hash(&link.to_ascii_lowercase());
            if !written.contains(&link.to_ascii_lowercase())
                && !champion.contains_path(link)
                && maps_holding(maps, hash).is_empty()
            {
                finding.missing_links.push(format!("{character}: {link}"));
            }
        }
    }
}

pub(super) fn valid_texture_bytes(bytes: &[u8]) -> bool {
    (bytes.len() >= 12
        && bytes.starts_with(b"TEX\0")
        && u16::from_le_bytes([bytes[4], bytes[5]]) > 0
        && u16::from_le_bytes([bytes[6], bytes[7]]) > 0)
        || (bytes.len() >= 128 && bytes.starts_with(b"DDS "))
        || bytes.starts_with(b"\x89PNG\r\n\x1a\n")
        || bytes.starts_with(b"\xFF\xD8\xFF")
        || (bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP")
}

pub(super) fn has_skin_numbered_ref(value: &str, character: &str) -> bool {
    for segment in value.split('_') {
        let low = segment.to_ascii_lowercase();
        if let Some(at) = low.rfind("skin") {
            let after = &low[at + 4..];
            let digit_len = after.bytes().take_while(u8::is_ascii_digit).count();
            let (digits, tail, head) = (&after[..digit_len], &after[digit_len..], &low[..at]);
            if !digits.is_empty()
                && digits.bytes().any(|b| b != b'0')
                && head.bytes().all(|b| b.is_ascii_alphabetic())
                && tail.bytes().all(|b| b.is_ascii_alphabetic())
            {
                return true;
            }
        }
    }
    let lower = value.to_ascii_lowercase();
    let char_low = character.to_ascii_lowercase();
    let plain = char_low.strip_prefix("jade_").unwrap_or(&char_low);
    let needle = format!("{plain}skin");
    let mut from = 0;
    while let Some(pos) = lower[from..].find(&needle) {
        let after = from + pos + needle.len();
        if lower
            .as_bytes()
            .get(after)
            .is_some_and(|b| b.is_ascii_digit() && *b != b'0')
        {
            return true;
        }
        from = after;
    }
    false
}

fn check_single_generated_file(path: &str, bytes: &[u8], finding: &mut SkinFinding) {
    if path.ends_with(".skn") {
        if bytes.get(0..4) != Some(&0x0011_2233u32.to_le_bytes()) {
            finding
                .stale_references
                .push(format!("{path}: invalid .skn magic"));
            return;
        }
        let major = bytes
            .get(4..6)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .unwrap_or(0);
        if !(1..=4).contains(&major) {
            finding
                .stale_references
                .push(format!("{path}: unsupported .skn major {major}"));
            return;
        }
        if major == 4 {
            match dekan_classic::skinned_mesh::parse(bytes) {
                Ok(mesh) => {
                    if let Err(e) = mesh.validate_submesh_indices() {
                        finding.stale_references.push(format!("{path}: {e}"));
                    }
                    if mesh.submeshes.is_empty()
                        || mesh.vertices.is_empty()
                        || mesh.indices.is_empty()
                        || !(52..=72).contains(&mesh.vertex_size)
                    {
                        finding
                            .stale_references
                            .push(format!("{path}: degenerate .skn geometry"));
                    }
                }
                Err(e) => finding.stale_references.push(format!("{path}: {e}")),
            }
        }
    } else if path.ends_with(".skl") {
        if let Ok(skl) = dekan_classic::skeleton::parse(bytes) {
            let len = skl.joints.len() as i16;
            if skl.joints.is_empty() || skl.joints.iter().any(|j| j.parent < -1 || j.parent >= len)
            {
                finding
                    .stale_references
                    .push(format!("{path}: invalid skeleton joint hierarchy"));
            }
        }
    } else if path.ends_with(".anm") {
        if bytes.len() < 12 || (!bytes.starts_with(b"r3d2anmd") && !bytes.starts_with(b"r3d2canm"))
        {
            finding
                .stale_references
                .push(format!("{path}: invalid .anm header"));
        }
    } else if path.ends_with(".tex")
        || path.ends_with(".dds")
        || path.ends_with(".png")
        || path.ends_with(".jpg")
        || path.ends_with(".jpeg")
        || path.ends_with(".webp")
    {
        if !valid_texture_bytes(bytes) {
            finding
                .stale_references
                .push(format!("{path}: invalid texture header"));
        }
    } else if path.ends_with(".bnk") {
        if !bytes.starts_with(b"BKHD") || dekan_classic::sound_bank::parse(bytes).is_err() {
            finding
                .stale_references
                .push(format!("{path}: invalid .bnk soundbank"));
        }
    } else if path.ends_with(".wpk") {
        if bytes.len() < 12 || !bytes.starts_with(b"r3d2") {
            finding
                .stale_references
                .push(format!("{path}: invalid .wpk header"));
        }
    } else if path.ends_with(".wem") {
        if bytes.len() < 12 || !bytes.starts_with(b"RIFF") {
            finding
                .stale_references
                .push(format!("{path}: invalid .wem header"));
        }
    } else if path.ends_with(".bin") {
        match dekan_wad::prop::parse_prop_file(bytes) {
            Ok(prop) => {
                for entry in &prop.entries {
                    if dekan_wad::prop::tree::parse_fields(&entry.body).is_err() {
                        finding.stale_references.push(format!(
                            "{path}: entry {:08x} failed field parse",
                            entry.key_hash
                        ));
                    }
                }
            }
            Err(e) => finding
                .stale_references
                .push(format!("{path}: invalid PROP bin: {e}")),
        }
    } else if (path.ends_with(".scb") && !bytes.starts_with(b"r3d2Mesh"))
        || (path.ends_with(".sco") && !bytes.starts_with(b"[ObjectBegin]"))
        || (path.ends_with(".mapgeo") && !bytes.starts_with(b"OEGM"))
        || (path.ends_with(".nvr") && !bytes.starts_with(b"NVR\0"))
        || (path.ends_with(".wgeo") && !bytes.starts_with(b"WGEO"))
        || (path.ends_with(".stringtable") && !bytes.starts_with(b"RST"))
        || ((path.ends_with(".gmesh")
            || path.ends_with(".tmesh")
            || path.ends_with(".troybin")
            || path.ends_with(".aimesh_ngrid")
            || path.ends_with(".ngrid_overlay")
            || path.ends_with(".rg_overlay"))
            && bytes.is_empty())
    {
        finding
            .stale_references
            .push(format!("{path}: invalid format header or empty payload"));
    }
}

pub(super) fn check_generated_meshes(
    wad_root: &Path,
    written: &HashSet<String>,
    finding: &mut SkinFinding,
) {
    let mut seen_hashes = HashSet::with_capacity(written.len());
    for path in written {
        let hash = wad_path_hash(path);
        if hash == 0 || !seen_hashes.insert(hash) {
            finding.stale_references.push(format!(
                "{path}: invalid or colliding WAD path hash {hash:016x}"
            ));
        }
        let Ok(bytes) = std::fs::read(wad_root.join(path)) else {
            continue;
        };
        check_single_generated_file(path, &bytes, finding);
    }
}

pub(super) fn generated_paths(wad_root: &Path) -> Vec<String> {
    let mut paths = Vec::new();
    let mut stack = vec![wad_root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Ok(relative) = path.strip_prefix(wad_root) {
                paths.push(
                    relative
                        .to_string_lossy()
                        .replace('\\', "/")
                        .to_ascii_lowercase(),
                );
            }
        }
    }
    paths.sort();
    paths
}
