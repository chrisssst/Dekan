use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use dekan_platform::fs::atomic_write;
use dekan_platform::panel::{PanelSkinStat, PanelStats};
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

const STATS_FILE: &str = "skin_stats.json";
const TOP_SKINS: usize = 3;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkinRecord {
    pub skin_id: u32,
    pub champion_name: String,
    pub skin_name: String,
    pub wins: u32,
    pub losses: u32,
    pub last_played: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct MatchStats {
    pub wins: u32,
    pub losses: u32,
    #[serde(default)]
    pub skins: BTreeMap<u32, SkinRecord>,
}

fn stats_path(state_dir: &Path) -> PathBuf {
    state_dir.join(STATS_FILE)
}

#[must_use]
pub fn winrate_pct(wins: u32, losses: u32) -> u8 {
    let total = wins.saturating_add(losses);
    if total == 0 {
        return 0;
    }
    let rounded = (u64::from(wins) * 100 + u64::from(total) / 2) / u64::from(total);
    u8::try_from(rounded.min(100)).unwrap_or(100)
}

#[must_use]
pub fn load(state_dir: &Path) -> MatchStats {
    let path = stats_path(state_dir);
    let Ok(bytes) = std::fs::read(&path) else {
        return MatchStats::default();
    };
    match serde_json::from_slice(&bytes) {
        Ok(stats) => stats,
        Err(e) => {
            warn!(path = %path.display(), error = %e, "Skin stats file could not be parsed; starting fresh");
            MatchStats::default()
        }
    }
}

pub fn record_match(
    state_dir: &Path,
    skin_id: u32,
    champion_name: &str,
    skin_name: &str,
    won: bool,
) {
    let mut stats = load(state_dir);
    if won {
        stats.wins = stats.wins.saturating_add(1);
    } else {
        stats.losses = stats.losses.saturating_add(1);
    }
    if skin_id > 0 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let entry = stats.skins.entry(skin_id).or_insert_with(|| SkinRecord {
            skin_id,
            champion_name: champion_name.to_owned(),
            skin_name: skin_name.to_owned(),
            wins: 0,
            losses: 0,
            last_played: now,
        });
        if !skin_name.is_empty() {
            entry.skin_name = skin_name.to_owned();
        }
        if !champion_name.is_empty() {
            entry.champion_name = champion_name.to_owned();
        }
        if won {
            entry.wins = entry.wins.saturating_add(1);
        } else {
            entry.losses = entry.losses.saturating_add(1);
        }
        entry.last_played = now;
    }
    if let Ok(json) = serde_json::to_vec_pretty(&stats) {
        if let Err(e) = atomic_write(&stats_path(state_dir), &json, false) {
            warn!(error = %e, "Could not save skin match stats");
        } else {
            debug!(
                skin_id,
                won,
                total_wins = stats.wins,
                total_losses = stats.losses,
                "Match stats recorded"
            );
        }
    }
}

#[must_use]
pub fn load_summary(state_dir: &Path) -> PanelStats {
    let stats = load(state_dir);
    let mut ranked: Vec<&SkinRecord> = stats.skins.values().collect();
    ranked.sort_by(|a, b| {
        let ga = a.wins.saturating_add(a.losses);
        let gb = b.wins.saturating_add(b.losses);
        gb.cmp(&ga)
            .then_with(|| winrate_pct(b.wins, b.losses).cmp(&winrate_pct(a.wins, a.losses)))
            .then_with(|| b.last_played.cmp(&a.last_played))
    });
    let top = ranked
        .into_iter()
        .take(TOP_SKINS)
        .map(|rec| {
            let name = if !rec.skin_name.is_empty() {
                rec.skin_name.clone()
            } else if !rec.champion_name.is_empty() {
                format!("{} #{}", rec.champion_name, rec.skin_id % 1000)
            } else {
                format!("Skin #{}", rec.skin_id)
            };
            PanelSkinStat {
                name,
                wins: rec.wins,
                losses: rec.losses,
                winrate: winrate_pct(rec.wins, rec.losses),
            }
        })
        .collect();
    PanelStats {
        wins: stats.wins,
        losses: stats.losses,
        winrate: winrate_pct(stats.wins, stats.losses),
        top,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_wins_losses_and_ranks_top_skins() {
        let dir = std::env::temp_dir().join(format!("dekan-stats-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir); // ignore-ok: test temp dir cleanup
        std::fs::create_dir_all(&dir).unwrap();

        record_match(&dir, 238031, "Zed", "Debonair Zed", true);
        record_match(&dir, 238031, "Zed", "Debonair Zed", true);
        record_match(&dir, 238031, "Zed", "Debonair Zed", false);
        record_match(&dir, 157009, "Yasuo", "Nightbringer Yasuo", true);

        let summary = load_summary(&dir);
        assert_eq!(summary.wins, 3);
        assert_eq!(summary.losses, 1);
        assert_eq!(summary.winrate, 75);
        assert_eq!(summary.top.len(), 2);
        assert_eq!(summary.top[0].name, "Debonair Zed");
        assert_eq!(summary.top[0].winrate, 67);
        assert_eq!(summary.top[1].name, "Nightbringer Yasuo");
        assert_eq!(summary.top[1].winrate, 100);

        let _ = std::fs::remove_dir_all(&dir); // ignore-ok: test temp dir teardown
    }
}
