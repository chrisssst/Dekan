use super::*;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct GameLogFacts {
    pub roster: Vec<(String, u32, bool)>,
    pub errors: BTreeMap<String, usize>,
}

#[must_use]
pub fn parse_game_log(content: &str) -> GameLogFacts {
    let mut facts = GameLogFacts::default();
    let mut seen = BTreeSet::new();
    for line in content.lines() {
        let body = line.split_once('|').map_or(line, |(_, rest)| rest).trim();
        if body.contains("CONNECTION READY") {
            let champion = body
                .split_once("Champion(")
                .and_then(|(_, r)| r.split_once(')'))
                .map(|(c, _)| c.to_owned());
            let skin = body
                .split_once("SkinID(")
                .and_then(|(_, r)| r.split_once(')'))
                .and_then(|(s, _)| s.parse::<u32>().ok());
            if let (Some(champion), Some(skin)) = (champion, skin) {
                let local = body.contains("**LOCAL**");
                if seen.insert((champion.clone(), skin, local)) {
                    facts.roster.push((champion, skin, local));
                }
            }
        } else if let Some(message) = body.strip_prefix("ERROR|") {
            *facts.errors.entry(message.trim().to_owned()).or_default() += 1;
        } else if let Some(message) = body.strip_prefix("FATAL|") {
            *facts
                .errors
                .entry(format!("FATAL {}", message.trim()))
                .or_default() += 1;
        }
    }
    facts
}

pub(super) fn newest_game_log(
    logs_dir: &Path,
    since: SystemTime,
) -> std::io::Result<Option<PathBuf>> {
    let mut newest: Option<(SystemTime, PathBuf)> = None;
    let mut skipped = 0usize;
    for dir in std::fs::read_dir(logs_dir)? {
        let Ok(dir) = dir else {
            skipped += 1;
            continue;
        };
        let Ok(files) = std::fs::read_dir(dir.path()) else {
            skipped += 1;
            continue;
        };
        for file in files {
            let Ok(file) = file else {
                skipped += 1;
                continue;
            };
            let path = file.path();
            if !path.to_string_lossy().ends_with("_r3dlog.txt") {
                continue;
            }
            let Ok(modified) = file.metadata().and_then(|m| m.modified()) else {
                skipped += 1;
                continue;
            };
            if modified >= since && newest.as_ref().is_none_or(|(t, _)| modified > *t) {
                newest = Some((modified, path));
            }
        }
    }
    if skipped > 0 {
        debug!(skipped, dir = %logs_dir.display(), "Game log folders or files could not be inspected");
    }
    Ok(newest.map(|(_, path)| path))
}

#[must_use]
pub fn game_logs_dir(game_dir: &Path) -> Option<PathBuf> {
    game_dir
        .parent()
        .map(|install| install.join("Logs").join("GameLogs"))
}

pub(super) fn report_game_log(game_dir: &Path, since: SystemTime) {
    let Some(dir) = game_logs_dir(game_dir) else {
        warn!(game_dir = %game_dir.display(), "Game log not read: the game folder has no parent");
        return;
    };
    let path = match newest_game_log(&dir, since) {
        Ok(Some(path)) => path,
        Ok(None) => {
            info!(dir = %dir.display(), "No game log was written for this match");
            return;
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            info!(dir = %dir.display(), "The game has no log folder; nothing to read for this match");
            return;
        }
        Err(e) => {
            warn!(dir = %dir.display(), error = %e, "Game log folder could not be read");
            return;
        }
    };
    let content = match std::fs::metadata(&path) {
        Ok(meta) if meta.len() <= MAX_GAME_LOG_BYTES => std::fs::read(&path),
        Ok(meta) => {
            warn!(file = %path.display(), bytes = meta.len(), "Game log too large to read; skipped");
            return;
        }
        Err(e) => Err(e),
    };
    let content = match content {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(e) => {
            warn!(file = %path.display(), error = %e, "Game log could not be read");
            return;
        }
    };
    let facts = parse_game_log(&content);
    crate::game::game_log_trace::trace(&content);
    info!(
        file = %path.display(),
        roster = ?facts.roster.iter().map(|(c, s, l)| format!("{c}:{s}{}", if *l { " (you)" } else { "" })).collect::<Vec<_>>(),
        distinct_errors = facts.errors.len(),
        "Game log: skins the game loaded for this match"
    );
    let mut errors: Vec<(&String, &usize)> = facts.errors.iter().collect();
    errors.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
    if errors.len() > MAX_LOGGED_ERRORS {
        info!(
            omitted = errors.len() - MAX_LOGGED_ERRORS,
            "Game log: more distinct errors than are listed; the most frequent follow"
        );
    }
    for (message, count) in errors.into_iter().take(MAX_LOGGED_ERRORS) {
        let count = *count;
        if message.contains("FATAL")
            || message.contains("mount failed")
            || message.contains("corrupt")
        {
            warn!(count, message = %message, "Game log error");
        } else {
            info!(count, message = %message, "Game log error");
        }
    }
}

#[must_use]
pub fn match_screenshots(
    install_dir: &Path,
    since: SystemTime,
    until: SystemTime,
) -> Vec<(SystemTime, PathBuf)> {
    let entries = match std::fs::read_dir(install_dir.join("Screenshots")) {
        Ok(entries) => entries,
        Err(e) => {
            if e.kind() != std::io::ErrorKind::NotFound {
                debug!(error = %e, "Match screenshots folder could not be read");
            }
            return Vec::new();
        }
    };
    let mut shots: Vec<(SystemTime, PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let image = path
                .extension()
                .and_then(|x| x.to_str())
                .is_some_and(|x| x.eq_ignore_ascii_case("png") || x.eq_ignore_ascii_case("jpg"));
            let modified = e.metadata().and_then(|m| m.modified()).ok()?;
            (image && modified >= since && modified <= until).then_some((modified, path))
        })
        .collect();
    shots.sort();
    if shots.len() > MAX_MATCH_SCREENSHOTS {
        shots.drain(..shots.len() - MAX_MATCH_SCREENSHOTS);
    }
    shots
}

pub(super) fn prune_exports(logs_dir: &Path) {
    let entries = match std::fs::read_dir(logs_dir) {
        Ok(entries) => entries,
        Err(e) => {
            warn!(dir = %logs_dir.display(), error = %e, "Old diagnostics exports could not be listed");
            return;
        }
    };
    let mut exports: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("dekan-diagnostics-") && n.ends_with(".zip"))
        })
        .collect();
    exports.sort();
    if exports.len() <= KEPT_AUTOMATIC_EXPORTS {
        return;
    }
    let mut failed = 0usize;
    let mut first_error = None;
    for old in &exports[..exports.len() - KEPT_AUTOMATIC_EXPORTS] {
        if let Err(e) = std::fs::remove_file(old) {
            failed += 1;
            first_error.get_or_insert_with(|| format!("{}: {e}", old.display()));
        }
    }
    if let Some(first_error) = first_error {
        warn!(failed, first_error = %first_error, "Old diagnostics exports not removed");
    }
}

pub(super) fn close_match(
    game_dir: &Path,
    logs_dir: Option<&Path>,
    since: SystemTime,
    timeline: &[(SystemTime, f64)],
) {
    report_game_log(game_dir, since);
    let until = SystemTime::now();
    let shots = game_dir
        .parent()
        .map(|install| match_screenshots(install, since, until))
        .unwrap_or_default();
    for (taken, path) in &shots {
        info!(
            file = %path.display(),
            game_time = ?game_time_for(timeline, *taken).map(format_game_time),
            "Screenshot taken during the match"
        );
    }
    let Some(logs_dir) = logs_dir else {
        return;
    };
    let images: Vec<PathBuf> = shots.into_iter().map(|(_, p)| p).collect();
    match crate::control_panel::export_diagnostics(logs_dir, until, &images) {
        Ok(zip) => info!(
            file = %zip.display(),
            screenshots = images.len(),
            "Match diagnostics exported automatically"
        ),
        Err(e) => warn!(error = %e, "Match diagnostics could not be exported"),
    }
    prune_exports(logs_dir);
}
