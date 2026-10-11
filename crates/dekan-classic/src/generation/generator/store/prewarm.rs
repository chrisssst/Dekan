use super::*;

pub(crate) type SharedScan = std::sync::Arc<std::sync::OnceLock<BTreeSet<String>>>;

pub(crate) fn shared_scan(alias: &str, stamp: &str) -> SharedScan {
    static SCANS: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, SharedScan>>,
    > = std::sync::OnceLock::new();
    if stamp.is_empty() {
        return SharedScan::default();
    }
    let prefix = format!("{}|", alias.to_ascii_lowercase());
    let key = format!("{prefix}{stamp}");
    let scans = SCANS.get_or_init(Default::default);
    let mut map = match scans.lock() {
        Ok(map) => map,
        Err(poisoned) => poisoned.into_inner(),
    };
    map.retain(|k, _| !k.starts_with(&prefix) || *k == key);
    std::sync::Arc::clone(map.entry(key).or_default())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrewarmGate {
    Go,
    Wait,
    Stop,
}

pub(crate) const PREWARM_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

#[must_use]
pub fn champion_aliases(game_dir: &Path) -> Vec<String> {
    let dir = game_dir.join("DATA").join("FINAL").join("Champions");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut aliases: Vec<String> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let alias = name.strip_suffix(".wad.client")?;
            (!alias.contains('.') && is_safe_alias(alias)).then(|| alias.to_owned())
        })
        .collect();
    aliases.sort_unstable();
    aliases
}

#[must_use]
pub fn companion_cache_is_current(game_dir: &Path, cache_dir: &Path, alias: &str) -> bool {
    let wad = game_dir
        .join("DATA")
        .join("FINAL")
        .join("Champions")
        .join(format!("{alias}.wad.client"));
    let stamp = wad_stamp(&wad);
    let cache = cache_dir.join(format!(
        "companion_names_{}.json",
        alias.to_ascii_lowercase()
    ));
    !stamp.is_empty()
        && std::fs::read(cache)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<CharacterCache>(&bytes).ok())
            .is_some_and(|cached| cached.source == stamp)
}

pub fn prewarm_companions(game_dir: &Path, cache_dir: &Path, gate: impl Fn() -> PrewarmGate) {
    let started = std::time::Instant::now();
    let aliases = champion_aliases(game_dir);
    let mut indexed = 0usize;
    'champions: for alias in &aliases {
        loop {
            match gate() {
                PrewarmGate::Go => break,
                PrewarmGate::Wait => std::thread::sleep(PREWARM_WAIT),
                PrewarmGate::Stop => break 'champions,
            }
        }
        if companion_cache_is_current(game_dir, cache_dir, alias) {
            indexed += 1;
            continue;
        }
        match StandardChampion::open(game_dir, alias) {
            Ok(champion) => {
                champion.with_cache_dir(cache_dir).scanned_names(true);
                indexed += 1;
            }
            Err(e) => debug!(alias, error = %e, "Champion not indexed ahead of time"),
        }
    }
    info!(
        champions = aliases.len(),
        indexed,
        elapsed_s = started.elapsed().as_secs(),
        "Companion characters indexed ahead of champion select"
    );
}
