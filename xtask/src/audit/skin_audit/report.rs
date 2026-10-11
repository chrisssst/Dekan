use super::*;

#[must_use]
pub fn render(reports: &[ChampionReport], missed: &BTreeMap<String, BTreeSet<String>>) -> String {
    let skins: usize = reports.iter().map(|r| r.skins.len()).sum();
    let clean: usize = reports
        .iter()
        .flat_map(|r| r.skins.values())
        .filter(|f| f.is_clean())
        .count();
    let with_companions = reports
        .iter()
        .filter(|r| !r.companion_skins.is_empty())
        .count();

    let mut lines = vec![
        "# Skin audit".to_owned(),
        String::new(),
        format!(
            "- champions: {} | skins and chromas generated: {skins} | clean: {clean} | with findings: {}",
            reports.len(),
            skins - clean
        ),
        format!("- champions with companion characters: {with_companions}"),
        String::new(),
        "## Companion characters per champion".to_owned(),
        String::new(),
        "| Champion | Companion | Skin numbers with their own bin |".to_owned(),
        "| --- | --- | --- |".to_owned(),
    ];
    for report in reports {
        for (companion, numbers) in &report.companion_skins {
            lines.push(format!("| {} | {companion} | {numbers:?} |", report.alias));
        }
    }
    lines.extend([
        String::new(),
        "## Findings per skin".to_owned(),
        String::new(),
        "| Champion | Skin | Build error | Source links dropped | Stale references | Missing links | Paths also written into a map WAD |"
            .to_owned(),
        "| --- | --- | --- | --- | --- | --- | --- |".to_owned(),
    ]);
    for report in reports {
        if let Some(error) = &report.open_error {
            lines.push(format!(
                "| {} | - | WAD not opened: {error} | | | | |",
                report.alias
            ));
            continue;
        }
        for (skin, finding) in report
            .skins
            .iter()
            .filter(|(_, f)| !f.is_clean() || !f.shared_with_map.is_empty())
        {
            lines.push(format!(
                "| {} | {skin} | {} | {} | {} | {} | {} |",
                report.alias,
                finding.build_error.as_deref().unwrap_or(""),
                if finding.source_links_dropped {
                    "yes"
                } else {
                    ""
                },
                finding.stale_references.join("; "),
                finding.missing_links.join("; "),
                finding.shared_with_map.join(", ")
            ));
        }
    }
    let forms: Vec<String> = reports
        .iter()
        .flat_map(|report| {
            report.forms.iter().filter(|(_, f)| !f.is_clean()).map(
                move |((skin, form), finding)| {
                    format!(
                        "| {} | {skin} | {form} | {} | {} | {} |",
                        report.alias,
                        finding.build_error.as_deref().unwrap_or(""),
                        finding.stale_references.join("; "),
                        finding.missing_links.join("; ")
                    )
                },
            )
        })
        .collect();
    if reports.iter().any(|r| !r.forms.is_empty()) {
        lines.extend([
            String::new(),
            "## Findings per form".to_owned(),
            String::new(),
        ]);
        if forms.is_empty() {
            lines.push("None.".to_owned());
        } else {
            lines.push(
                "| Champion | Skin | Form | Build error | Stale references | Missing links |"
                    .to_owned(),
            );
            lines.push("| --- | --- | --- | --- | --- | --- |".to_owned());
            lines.extend(forms);
        }
    }
    lines.extend(crate::audit::form_audit::render_form_cycles(reports));
    lines.extend([
        String::new(),
        "## Characters with skin bins that the scan did not attach".to_owned(),
        String::new(),
    ]);
    if missed.is_empty() {
        lines.push("None.".to_owned());
    }
    for (alias, names) in missed {
        lines.push(format!("- {alias}: {names:?}"));
    }
    lines.push(String::new());
    lines.join("\n")
}

#[must_use]
pub fn default_report_path() -> PathBuf {
    std::env::temp_dir().join("dekan_skin_audit.md")
}
