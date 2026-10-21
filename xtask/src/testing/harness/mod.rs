use crate::game_dir;

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use dekan_classic::client_data::ClientGameData;
use dekan_classic::generator::{ClassicChampion, StandardChampion, slots_for};
use dekan_inject::overlay_builder;
use dekan_lcu::champion_assets::ChampionAssets;
use dekan_wad::wad::WadFile;

#[derive(Debug, Clone)]
pub enum Case {
    Standard {
        alias: String,
        skin: u32,
        base: Option<u32>,
        label: String,
    },
    Classic {
        alias: String,
        classic_alias: String,
        skin: u32,
        label: String,
    },
}

impl Case {
    fn label(&self) -> &str {
        match self {
            Self::Standard { label, .. } | Self::Classic { label, .. } => label,
        }
    }
}

#[derive(Debug, Default)]
pub struct Outcome {
    pub label: String,
    pub failures: Vec<String>,
    pub overlay_wads: usize,
    pub changed_entries: usize,
    pub verbatim_entries: usize,
    pub build_ms: u128,
}

fn assets_of(data: &ClientGameData, id: u32) -> Option<ChampionAssets> {
    data.read(&format!("v1/champions/{id}.json"))
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
}

pub fn sample(data: &ClientGameData, game: &Path) -> Result<Vec<Case>, String> {
    let aliases = data.champion_aliases().map_err(|e| e.to_string())?;
    let mut cases = Vec::new();
    let mut skipped = Vec::new();
    for (id, alias) in aliases.iter().filter(|(id, _)| **id < 60_000) {
        let Some(assets) = assets_of(data, *id) else {
            skipped.push(format!("{alias}: no champion data in the client"));
            continue;
        };
        let champion = match StandardChampion::open(game, alias) {
            Ok(champion) => champion,
            Err(e) => {
                skipped.push(format!("{alias}: {e}"));
                continue;
            }
        };
        let offered: Vec<_> = assets
            .skins
            .iter()
            .filter(|s| !s.is_base && champion.has_skin(s.id % 1000))
            .collect();
        if let Some(latest) = offered.iter().max_by_key(|s| s.id % 1000) {
            cases.push(Case::Standard {
                alias: alias.clone(),
                skin: latest.id % 1000,
                base: None,
                label: format!("{alias} {} {}", latest.id, latest.name),
            });
        }
        if let Some((skin, chroma)) = offered
            .iter()
            .rev()
            .find_map(|s| s.chromas.last().map(|c| (s, c)))
            .filter(|(_, c)| champion.has_skin(c.id % 1000))
        {
            cases.push(Case::Standard {
                alias: alias.clone(),
                skin: chroma.id % 1000,
                base: Some(skin.id % 1000),
                label: format!("{alias} {} {}", chroma.id, chroma.name),
            });
        }
    }
    if !skipped.is_empty() {
        println!(
            "campeoes sem caso (dados ausentes ou jogo ilegivel): {} | primeiro: {}",
            skipped.len(),
            skipped[0]
        );
    }
    for (alias, skin, base, label) in [
        ("Orianna", 1, None, "reported: Orianna ball"),
        ("Zed", 10, None, "reported: Zed shadows, Galaxy Slayer"),
        ("Zed", 12, Some(10), "reported: Zed chroma 238012"),
        ("Seraphine", 2, Some(1), "tier: K/DA ALL OUT Rising Star"),
        ("Seraphine", 3, Some(1), "tier: K/DA ALL OUT Superstar"),
        (
            "Tristana",
            80,
            Some(79),
            "tier: Immortalized Legend Tristana",
        ),
        ("Ahri", 86, Some(85), "tier: Immortalized Legend Ahri"),
        ("Kaisa", 71, Some(70), "tier: Immortalized Legend Kai'Sa"),
        (
            "Garen",
            44,
            None,
            "legendary animation graph: God-King Garen",
        ),
        ("Lux", 7, None, "ultimate: Elementalist Lux"),
    ] {
        cases.push(Case::Standard {
            alias: alias.into(),
            skin,
            base,
            label: label.into(),
        });
    }
    for (alias, classic_alias, skin, label) in [
        (
            "Annie",
            "Jade_Annie",
            303,
            "classic: Classic Annie (Founder's Goth)",
        ),
        (
            "MonkeyKing",
            "Jade_Wukong",
            3,
            "classic: Jade Dragon Wukong",
        ),
    ] {
        cases.push(Case::Classic {
            alias: alias.into(),
            classic_alias: classic_alias.into(),
            skin,
            label: label.into(),
        });
    }
    Ok(cases)
}

#[must_use]
pub fn render(outcomes: &[Outcome]) -> String {
    let failed: Vec<&Outcome> = outcomes.iter().filter(|o| !o.failures.is_empty()).collect();
    let mut lines = vec![
        "# Pipeline harness".to_owned(),
        String::new(),
        format!(
            "- cases: {} | passed: {} | failed: {}",
            outcomes.len(),
            outcomes.len() - failed.len(),
            failed.len()
        ),
        format!(
            "- overlay build time: median {} ms, max {} ms",
            median(outcomes.iter().map(|o| o.build_ms).collect()),
            outcomes.iter().map(|o| o.build_ms).max().unwrap_or(0)
        ),
        String::new(),
        "| Case | Result | Overlay WADs | Changed | Verbatim | Build ms |".to_owned(),
        "| --- | --- | --- | --- | --- | --- |".to_owned(),
    ];
    for o in outcomes {
        lines.push(format!(
            "| {} | {} | {} | {} | {} | {} |",
            o.label,
            if o.failures.is_empty() { "ok" } else { "FAIL" },
            o.overlay_wads,
            o.changed_entries,
            o.verbatim_entries,
            o.build_ms
        ));
    }
    lines.extend([String::new(), "## Failures".to_owned(), String::new()]);
    for o in &failed {
        for failure in &o.failures {
            lines.push(format!("- {}: {failure}", o.label));
        }
    }
    lines.push(String::new());
    lines.join("\n")
}

fn median(mut values: Vec<u128>) -> u128 {
    values.sort_unstable();
    values.get(values.len() / 2).copied().unwrap_or(0)
}

pub(crate) fn run_harness(args: &[String]) {
    let mut root = None;
    let mut out = std::env::temp_dir().join("dekan_harness.md");
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--root" => root = iter.next().map(PathBuf::from),
            "--out" => {
                if let Some(path) = iter.next() {
                    out = PathBuf::from(path);
                }
            }
            other => println!("argumento ignorado: {other}"),
        }
    }
    let Some(game) = root.or_else(game_dir) else {
        return;
    };
    let data = match dekan_classic::client_data::ClientGameData::for_game(&game) {
        Ok(data) => data,
        Err(e) => {
            eprintln!("[ERRO] dados do cliente indisponiveis: {e}");
            std::process::exit(1);
        }
    };
    let cases = match sample(&data, &game) {
        Ok(cases) => cases,
        Err(e) => {
            eprintln!("[ERRO] {e}");
            std::process::exit(1);
        }
    };
    if cases.is_empty() {
        eprintln!("[ERRO] nenhum caso de teste foi montado a partir dos dados do cliente");
        std::process::exit(1);
    }
    let scratch = std::env::temp_dir().join("dekan_harness_run");
    let cache = std::env::temp_dir().join("dekan_harness_cache");
    if let Err(e) = std::fs::create_dir_all(&cache) {
        eprintln!("[ERRO] pasta temporaria indisponivel: {e}");
        std::process::exit(1);
    }
    println!("jogo: {} | casos: {}", game.display(), cases.len());
    let mut outcomes = Vec::with_capacity(cases.len());
    for (n, case) in cases.iter().enumerate() {
        let outcome = run_case(case, &game, &scratch, &cache);
        println!(
            "[{}/{}] {} {} ({} ms)",
            n + 1,
            cases.len(),
            if outcome.failures.is_empty() {
                "ok"
            } else {
                "FALHA"
            },
            outcome.label,
            outcome.build_ms
        );
        outcomes.push(outcome);
    }
    match std::fs::write(&out, render(&outcomes)) {
        Ok(()) => println!("relatorio: {}", out.display()),
        Err(e) => {
            eprintln!("[ERRO] relatorio nao gravado em {}: {e}", out.display());
            std::process::exit(1);
        }
    }
    if outcomes.iter().any(|o| !o.failures.is_empty()) {
        std::process::exit(1);
    }
}

mod run;

pub use run::run_case;
