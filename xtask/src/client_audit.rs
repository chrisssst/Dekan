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

#[must_use]
pub fn render(findings: &[ChampionFindings]) -> String {
    let total: usize = findings.iter().map(|f| f.entries).sum();
    let mut by_kind: BTreeMap<&str, usize> = BTreeMap::new();
    let kind_name = |kind: EntryKind| match kind {
        EntryKind::Skin => "skin",
        EntryKind::Chroma => "chroma",
        EntryKind::Tier => "tier",
    };
    for f in findings {
        for e in &f.not_in_catalog {
            *by_kind.entry(kind_name(e.kind)).or_default() += 1;
        }
    }
    let missing_data: usize = findings.iter().map(|f| f.no_game_data.len()).sum();
    let wrong_parent: usize = findings.iter().map(|f| f.wrong_parent.len()).sum();
    let alias_mismatch: Vec<&ChampionFindings> = findings
        .iter()
        .filter(|f| {
            f.offline_alias
                .as_deref()
                .is_none_or(|a| !a.eq_ignore_ascii_case(&f.alias))
        })
        .collect();

    let mut lines = vec![
        "# Client audit".to_owned(),
        String::new(),
        format!(
            "- champions: {} | client entries (skins, chromas, tiers): {total}",
            findings.len()
        ),
        format!("- listed by the client but not selectable in Dekan: {by_kind:?}"),
        format!("- selectable, but resolved to another parent skin: {wrong_parent}"),
        format!("- listed by the client, no skin file in the installed game: {missing_data}"),
        format!(
            "- champions whose archive Dekan cannot name without the client running: {}",
            alias_mismatch.len()
        ),
        String::new(),
        "## Not selectable in Dekan".to_owned(),
        String::new(),
        "| Champion | Id | Kind | Parent | Name |".to_owned(),
        "| --- | --- | --- | --- | --- |".to_owned(),
    ];
    for f in findings {
        for e in &f.not_in_catalog {
            lines.push(format!(
                "| {} | {} | {} | {} | {} |",
                f.alias,
                e.id,
                kind_name(e.kind),
                e.parent,
                e.name
            ));
        }
    }
    lines.extend([
        String::new(),
        "## Resolved to another parent".to_owned(),
        String::new(),
    ]);
    for f in findings {
        for (e, got) in &f.wrong_parent {
            lines.push(format!(
                "- {} {} ({}): client parent {}, Dekan parent {got}",
                f.alias,
                e.id,
                kind_name(e.kind),
                e.parent
            ));
        }
    }
    lines.extend([
        String::new(),
        "## Listed by the client, no skin file in the game".to_owned(),
        String::new(),
        "| Champion | Id | Kind | Parent | Name |".to_owned(),
        "| --- | --- | --- | --- | --- |".to_owned(),
    ]);
    for f in findings {
        for e in &f.no_game_data {
            lines.push(format!(
                "| {} | {} | {} | {} | {} |",
                f.alias,
                e.id,
                kind_name(e.kind),
                e.parent,
                e.name
            ));
        }
    }
    lines.extend([
        String::new(),
        "## Alias without the client running".to_owned(),
        String::new(),
    ]);
    for f in &alias_mismatch {
        lines.push(format!(
            "- {} {}: resolved {:?}",
            f.champion_id, f.alias, f.offline_alias
        ));
    }
    lines.extend([
        String::new(),
        "## Skin numbers in the game the client does not list".to_owned(),
        String::new(),
    ]);
    for f in findings.iter().filter(|f| !f.game_only_numbers.is_empty()) {
        lines.push(format!("- {}: {:?}", f.alias, f.game_only_numbers));
    }
    lines.extend([String::new(), "## Errors".to_owned(), String::new()]);
    for f in findings {
        if let Some(error) = &f.error {
            lines.push(format!("- {} {}: {error}", f.champion_id, f.alias));
        }
    }
    lines.push(String::new());
    lines.join("\n")
}

#[derive(Debug, Default)]
pub struct ClassicFindings {
    pub alias: String,
    pub error: Option<String>,
    pub client_numbers: std::collections::BTreeSet<u32>,
    pub offered_numbers: std::collections::BTreeSet<u32>,
    pub jade_numbers: std::collections::BTreeSet<u32>,
    pub client_names: BTreeMap<u32, String>,
    pub build_errors: Vec<String>,
}

pub fn audit_classic(
    data: &ClientData,
    game: &Path,
    classic_id: i64,
    classic_alias: &str,
    staging: &Path,
) -> ClassicFindings {
    use dekan_classic::generator::{ClassicChampion, slots_for};

    let mut findings = ClassicFindings {
        alias: classic_alias.to_owned(),
        ..ClassicFindings::default()
    };
    let (Ok(classic_u32), Ok(regular)) = (
        u32::try_from(classic_id),
        u32::try_from(classic_id - CLASSIC_CHAMPION_OFFSET),
    ) else {
        findings.error = Some("id out of range".into());
        return findings;
    };
    let read_json = |id: u32| {
        data.read(&format!("v1/champions/{id}.json"))
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
    };
    let Some(classic_raw) = read_json(classic_u32) else {
        findings.error = Some("classic champion JSON missing in the client".into());
        return findings;
    };
    for entry in client_entries(&classic_raw) {
        findings.client_numbers.insert(entry.id % 1000);
        findings.client_names.insert(entry.id % 1000, entry.name);
    }
    let regular_assets: Option<ChampionAssets> = data
        .read(&format!("v1/champions/{regular}.json"))
        .and_then(|bytes| serde_json::from_slice(&bytes).ok());
    let Some(alias) = regular_assets.as_ref().map(|a| a.alias.clone()) else {
        findings.error = Some("regular champion JSON missing in the client".into());
        return findings;
    };
    let champion = match ClassicChampion::open(game, &alias) {
        Ok(champion) => champion.with_client_character(Some(classic_alias)),
        Err(e) => {
            findings.error = Some(format!("champion WAD for '{alias}': {e}"));
            return findings;
        }
    };
    findings.jade_numbers = champion
        .skin_numbers(champion.main_character(), 1000)
        .into_iter()
        .collect();
    let classic_assets: Option<ChampionAssets> = data
        .read(&format!("v1/champions/{classic_u32}.json"))
        .and_then(|bytes| serde_json::from_slice(&bytes).ok());
    let (catalog, _) = dekan_app::catalog::build_classic_catalog(
        classic_u32,
        classic_assets.as_ref(),
        &findings.jade_numbers,
    );
    for skin in &catalog.skins {
        findings.offered_numbers.insert(skin.id % 1000);
        for chroma in &skin.chromas {
            findings.offered_numbers.insert(chroma.id % 1000);
        }
    }
    let known = champion.jade_names_from_bins_cached(staging);
    for number in &findings.offered_numbers {
        match champion.build_mod(*number, &slots_for(None), &known, staging) {
            Ok(folder) => {
                let _ = std::fs::remove_dir_all(staging.join(folder)); // ignore-ok: probe scratch folder
            }
            Err(e) => findings.build_errors.push(format!("{number}: {e}")),
        }
    }
    findings
}

#[must_use]
pub fn render_classic(findings: &[ClassicFindings]) -> String {
    let missing: usize = findings
        .iter()
        .map(|f| f.client_numbers.difference(&f.offered_numbers).count())
        .sum();
    let extra: usize = findings
        .iter()
        .map(|f| f.offered_numbers.difference(&f.client_numbers).count())
        .sum();
    let no_file: usize = findings
        .iter()
        .map(|f| f.client_numbers.difference(&f.jade_numbers).count())
        .sum();
    let client_total: usize = findings.iter().map(|f| f.client_numbers.len()).sum();
    let built: usize = findings.iter().map(|f| f.offered_numbers.len()).sum();
    let build_errors: Vec<String> = findings
        .iter()
        .flat_map(|f| {
            f.build_errors
                .iter()
                .map(move |e| format!("{} {e}", f.alias))
        })
        .collect();
    let mut lines =
        vec![
        "# Rift Classic audit".to_owned(),
        String::new(),
        format!(
            "- classic champions: {} | classic entries the client lists: {client_total}",
            findings.len()
        ),
        format!("- listed by the client for Classic, not offered by Dekan: {missing}"),
        format!("- offered by Dekan, not listed by the client for Classic: {extra}"),
        format!("- listed by the client, no jade skin file in the game: {no_file}"),
        format!("- Classic mods generated: {built}, failed: {}", build_errors.len()),
        String::new(),
        "| Champion | Not offered (client lists) | Offered (client does not list) | No jade file |"
            .to_owned(),
        "| --- | --- | --- | --- |".to_owned(),
    ];
    for f in findings {
        if let Some(error) = &f.error {
            lines.push(format!("| {} | error: {error} | | |", f.alias));
            continue;
        }
        let missing: Vec<String> = f
            .client_numbers
            .difference(&f.offered_numbers)
            .map(|n| format!("{n} {}", f.client_names.get(n).map_or("", String::as_str)))
            .collect();
        let extra: Vec<u32> = f
            .offered_numbers
            .difference(&f.client_numbers)
            .copied()
            .collect();
        let no_file: Vec<u32> = f
            .client_numbers
            .difference(&f.jade_numbers)
            .copied()
            .collect();
        if missing.is_empty() && extra.is_empty() && no_file.is_empty() {
            continue;
        }
        lines.push(format!(
            "| {} | {} | {extra:?} | {no_file:?} |",
            f.alias,
            missing.join("; ")
        ));
    }
    lines.extend(
        build_errors
            .into_iter()
            .map(|e| format!("- build failed: {e}")),
    );
    lines.push(String::new());
    lines.join("\n")
}
