use crate::game_dir;
use std::path::PathBuf;

use std::collections::BTreeMap;
use std::path::Path;

use dekan_app::catalog::build_catalog;
use dekan_classic::generator::StandardChampion;
use dekan_core::library::ChampionLibrary;
use dekan_lcu::champion_assets::ChampionAssets;
use serde_json::Value;

const CLASSIC_CHAMPION_OFFSET: i64 = 60_000;

pub use dekan_classic::client_data::ClientGameData as ClientData;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EntryKind {
    Skin,
    Chroma,
    Tier,
}

#[derive(Debug, Clone)]
pub struct ClientEntry {
    pub id: u32,
    pub parent: u32,
    pub kind: EntryKind,
    pub name: String,
}

#[derive(Debug, Default)]
pub struct ChampionFindings {
    pub champion_id: i64,
    pub alias: String,
    pub error: Option<String>,
    pub entries: usize,
    pub not_in_catalog: Vec<ClientEntry>,
    pub wrong_parent: Vec<(ClientEntry, u32)>,
    pub no_game_data: Vec<ClientEntry>,
    pub offline_alias: Option<String>,
    pub game_only_numbers: Vec<u32>,
}

fn as_u32(value: &Value) -> Option<u32> {
    value.as_u64().and_then(|v| u32::try_from(v).ok())
}

#[must_use]
pub fn client_entries(champion: &Value) -> Vec<ClientEntry> {
    let mut entries = Vec::new();
    let Some(skins) = champion.get("skins").and_then(Value::as_array) else {
        return entries;
    };
    for skin in skins {
        let Some(id) = skin.get("id").and_then(as_u32) else {
            continue;
        };
        if skin.get("isBase").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let name = |v: &Value| {
            v.get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        entries.push(ClientEntry {
            id,
            parent: id,
            kind: EntryKind::Skin,
            name: name(skin),
        });
        for chroma in skin
            .get("chromas")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(chroma_id) = chroma.get("id").and_then(as_u32) {
                entries.push(ClientEntry {
                    id: chroma_id,
                    parent: id,
                    kind: EntryKind::Chroma,
                    name: name(chroma),
                });
            }
        }
        for tier in skin
            .pointer("/questSkinInfo/tiers")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(tier_id) = tier.get("id").and_then(as_u32).filter(|t| *t != id) {
                entries.push(ClientEntry {
                    id: tier_id,
                    parent: id,
                    kind: EntryKind::Tier,
                    name: name(tier),
                });
            }
        }
    }
    entries
}

pub fn champion_ids(data: &ClientData) -> Result<Vec<(i64, String)>, String> {
    let bytes = data
        .read("v1/champion-summary.json")
        .ok_or("champion-summary.json not in the client")?;
    let summary: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    Ok(summary
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| Some((c.get("id")?.as_i64()?, c.get("alias")?.as_str()?.to_owned())))
        .filter(|(id, _)| *id > 0)
        .collect())
}

pub fn audit_champion(
    data: &ClientData,
    game: &Path,
    champion_id: i64,
    summary_alias: &str,
) -> ChampionFindings {
    let mut findings = ChampionFindings {
        champion_id,
        alias: summary_alias.to_owned(),
        ..ChampionFindings::default()
    };
    let Some(bytes) = data.read(&format!("v1/champions/{champion_id}.json")) else {
        findings.error = Some("champion JSON missing in the client".into());
        return findings;
    };
    let raw: Value = match serde_json::from_slice(&bytes) {
        Ok(raw) => raw,
        Err(e) => {
            findings.error = Some(format!("champion JSON unreadable: {e}"));
            return findings;
        }
    };
    let assets: ChampionAssets = match serde_json::from_slice(&bytes) {
        Ok(assets) => assets,
        Err(e) => {
            findings.error = Some(format!("Dekan cannot parse the champion JSON: {e}"));
            return findings;
        }
    };
    let Ok(regular_id) = u32::try_from(champion_id) else {
        findings.error = Some("negative id".into());
        return findings;
    };
    findings.offline_alias = dekan_classic::generator::resolve_alias_with_id(
        game,
        None,
        Some(regular_id),
        &std::env::temp_dir().join("dekan_client_audit_no_library"),
    );

    let entries = client_entries(&raw);
    findings.entries = entries.len();
    let catalog = build_catalog(
        &ChampionLibrary {
            champion_id: regular_id,
            skins: Vec::new(),
        },
        Some(&assets),
    );
    for entry in &entries {
        match catalog.resolve_target(entry.id) {
            None => findings.not_in_catalog.push(entry.clone()),
            Some(target) if target.skin_id != entry.parent => {
                findings.wrong_parent.push((entry.clone(), target.skin_id));
            }
            Some(_) => {}
        }
    }

    match StandardChampion::open(game, &assets.alias) {
        Ok(champion) => {
            let numbers: std::collections::BTreeSet<u32> =
                champion.skin_numbers(1000).into_iter().collect();
            for entry in &entries {
                if !numbers.contains(&(entry.id % 1000)) {
                    findings.no_game_data.push(entry.clone());
                }
            }
            let listed: std::collections::BTreeSet<u32> =
                entries.iter().map(|e| e.id % 1000).collect();
            findings.game_only_numbers = numbers
                .into_iter()
                .filter(|n| *n != 0 && !listed.contains(n))
                .collect();
        }
        Err(e) => findings.error = Some(format!("champion WAD for '{}': {e}", assets.alias)),
    }
    findings
}

#[must_use]
pub fn is_classic(champion_id: i64) -> bool {
    champion_id >= CLASSIC_CHAMPION_OFFSET
}

pub(crate) fn run_client_audit(args: &[String]) {
    let mut client_dir = None;
    let mut root = None;
    let mut out = std::env::temp_dir().join("dekan_client_audit.md");
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--client" => client_dir = iter.next().map(PathBuf::from),
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
    let client_dir = client_dir
        .or_else(|| game.parent().map(std::path::Path::to_path_buf))
        .unwrap_or_default();
    let data = match ClientData::open(&client_dir) {
        Ok(data) => data,
        Err(e) => {
            println!("dados do cliente indisponiveis: {e}");
            return;
        }
    };
    let champions = match champion_ids(&data) {
        Ok(champions) => champions,
        Err(e) => {
            println!("{e}");
            return;
        }
    };
    let regular: Vec<&(i64, String)> = champions
        .iter()
        .filter(|(id, _)| !is_classic(*id))
        .collect();
    println!(
        "cliente: {} | jogo: {} | campeoes: {} (+{} classicos)",
        client_dir.display(),
        game.display(),
        regular.len(),
        champions.len() - regular.len()
    );
    let findings: Vec<ChampionFindings> = regular
        .iter()
        .map(|(id, alias)| audit_champion(&data, &game, *id, alias))
        .collect();
    let staging = std::env::temp_dir().join("dekan_client_audit_classic");
    if let Err(e) = std::fs::create_dir_all(&staging) {
        println!("pasta temporaria indisponivel: {e}");
        return;
    }
    let classic: Vec<ClassicFindings> = champions
        .iter()
        .filter(|(id, _)| is_classic(*id))
        .map(|(id, alias)| audit_classic(&data, &game, *id, alias, &staging))
        .collect();
    let _ = std::fs::remove_dir_all(&staging); // ignore-ok: probe scratch folder
    let report = format!(
        "{}
{}",
        render(&findings),
        render_classic(&classic)
    );
    match std::fs::write(&out, report) {
        Ok(()) => println!("relatorio: {}", out.display()),
        Err(e) => println!("relatorio nao gravado em {}: {e}", out.display()),
    }
}

pub(crate) fn run_client_dump(args: &[String]) {
    let Some(client_dir) = args.first().map(PathBuf::from) else {
        println!("uso: client-dump <pasta do cliente> <caminho relativo>");
        return;
    };
    let data = match ClientData::open(&client_dir) {
        Ok(data) => data,
        Err(e) => {
            println!("{e}");
            return;
        }
    };
    for relative in &args[1..] {
        match data.read(relative) {
            Some(bytes) => println!("{}", String::from_utf8_lossy(&bytes)),
            None => println!("ausente: {relative}"),
        }
    }
}

mod classic;
mod report;

pub use classic::{ClassicFindings, audit_classic, render_classic};
pub use report::render;
