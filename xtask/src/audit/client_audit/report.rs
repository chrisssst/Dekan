use super::*;

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
