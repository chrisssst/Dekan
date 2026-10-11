use super::*;

#[must_use]
pub fn resolve_alias_with_id(
    game_dir: &Path,
    client_alias: Option<&str>,
    champion_id: Option<u32>,
    library_champion_dir: &Path,
) -> Option<String> {
    let champions = game_dir.join("DATA").join("FINAL").join("Champions");
    if let Some(alias) = client_alias.filter(|a| is_safe_alias(a)) {
        if champions.join(format!("{alias}.wad.client")).is_file() {
            return Some(alias.to_owned());
        }
    }
    if let Some(installed) =
        champion_id.and_then(|id| crate::client_data::champion_alias(game_dir, id))
    {
        if is_safe_alias(&installed) && champions.join(format!("{installed}.wad.client")).is_file()
        {
            return Some(installed);
        }
    }
    resolve_alias(game_dir, client_alias, library_champion_dir)
}

#[must_use]
pub fn resolve_alias(
    game_dir: &Path,
    client_alias: Option<&str>,
    library_champion_dir: &Path,
) -> Option<String> {
    let champions = game_dir.join("DATA").join("FINAL").join("Champions");
    if let Some(alias) = client_alias.filter(|a| is_safe_alias(a)) {
        if champions.join(format!("{alias}.wad.client")).is_file() {
            return Some(alias.to_owned());
        }
        warn!(
            alias,
            "Client alias has no champion WAD; trying the skin library"
        );
    }

    let mut archives: Vec<PathBuf> = Vec::new();
    let mut stack = vec![library_champion_dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("fantome") || e.eq_ignore_ascii_case("zip"))
            {
                archives.push(path);
            }
        }
    }
    archives.sort();

    for archive in archives {
        let Ok(file) = std::fs::File::open(&archive) else {
            continue;
        };
        match dekan_wad::fantome::wad_names_in_archive(std::io::BufReader::new(file)) {
            Ok(names) if names.len() == 1 => {
                if let Some(alias) = names.into_iter().next().filter(|a| is_safe_alias(a)) {
                    return Some(alias);
                }
            }
            Ok(names) => debug!(
                archive = %archive.display(),
                wads = names.len(),
                "Archive does not target exactly one champion WAD"
            ),
            Err(e) => debug!(archive = %archive.display(), error = %e, "Archive unreadable"),
        }
    }
    None
}
