use std::collections::BTreeMap;

use tracing::{Level, debug};

const KEYWORDS: [&str; 12] = [
    "anim", "clip", "submesh", "mesh", "skin", "gear", "toggle", "skeleton", "material",
    "particle", "vfx", "emote",
];

const MAX_DISTINCT: usize = 300;

#[must_use]
pub fn animation_lines(content: &str) -> BTreeMap<String, (String, usize)> {
    let mut found: BTreeMap<String, (String, usize)> = BTreeMap::new();
    for line in content.lines() {
        let (time, message) = line.split_once('|').unwrap_or(("", line));
        let lower = message.to_ascii_lowercase();
        if !KEYWORDS.iter().any(|k| lower.contains(k)) {
            continue;
        }
        let message = message.trim().to_owned();
        if let Some((_, count)) = found.get_mut(&message) {
            *count += 1;
        } else if found.len() < MAX_DISTINCT {
            found.insert(message, (time.trim().to_owned(), 1));
        }
    }
    found
}

pub fn trace(content: &str) {
    if !tracing::enabled!(Level::DEBUG) {
        return;
    }
    let lines = animation_lines(content);
    debug!(
        distinct = lines.len(),
        "Game log trace: animation, mesh and skin lines of the match"
    );
    for (message, (first_seen, count)) in &lines {
        debug!(first_seen = %first_seen, count, message = %message, "Game log trace");
    }
}

#[cfg(test)]
mod tests;
