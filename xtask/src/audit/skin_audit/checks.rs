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
