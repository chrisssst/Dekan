use super::*;

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
