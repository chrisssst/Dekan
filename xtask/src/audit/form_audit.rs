use dekan_classic::form_gear::{GearCoverage, ModelOutcome};

use super::skin_audit::ChampionReport;

fn limits(coverage: &GearCoverage) -> Vec<String> {
    let mut out = Vec::new();
    match &coverage.model {
        Some(ModelOutcome::Refused(reason)) => out.push(format!("no cycle: {reason}")),
        _ if coverage.mesh_swap => {
            out.push("no cycle: a form uses another mesh or skeleton".to_owned());
        }
        _ => {}
    }
    if !coverage.per_form_look.is_empty() {
        let look: Vec<&str> = coverage.per_form_look.iter().copied().collect();
        out.push(format!("first form's {}", look.join(", ")));
    }
    if coverage.script_states > 0 {
        out.push(format!(
            "{} skin script states (buffs the server applies)",
            coverage.script_states
        ));
    }
    out
}

fn model(coverage: &GearCoverage) -> String {
    match coverage.model {
        Some(ModelOutcome::Merged { forms, twins }) => {
            format!("{forms} forms, {twins} skinning twins")
        }
        _ => "-".to_owned(),
    }
}

#[must_use]
pub fn render_form_cycles(reports: &[ChampionReport]) -> Vec<String> {
    let cycles: Vec<(&str, u32, usize, &GearCoverage, Vec<String>)> = reports
        .iter()
        .flat_map(|report| {
            report
                .form_cycles
                .iter()
                .map(move |(skin, (forms, coverage))| {
                    (
                        report.alias.as_str(),
                        *skin,
                        *forms,
                        coverage,
                        limits(coverage),
                    )
                })
        })
        .collect();
    let limited = cycles.iter().filter(|c| !c.4.is_empty()).count();
    let merged = cycles
        .iter()
        .filter(|c| matches!(c.3.model, Some(ModelOutcome::Merged { .. })))
        .count();
    let mut lines = vec![
        String::new(),
        "## Ctrl+5 form cycles".to_owned(),
        String::new(),
        format!(
            "- skins with forms: {} | forms merged into one model: {merged} | with something that cannot follow Ctrl+5: {limited}",
            cycles.len()
        ),
        String::new(),
    ];
    if cycles.is_empty() {
        lines.push("None.".to_owned());
        return lines;
    }
    lines.push(
        "| Champion | Skin | Forms | One model | Forms with own idle | Parts with own material | Parts copied per form | Not carried |"
            .to_owned(),
    );
    lines.push("| --- | --- | --- | --- | --- | --- | --- | --- |".to_owned());
    for (alias, skin, forms, coverage, limits) in &cycles {
        let carried = if limits.is_empty() {
            "none".to_owned()
        } else {
            limits.join("; ")
        };
        lines.push(format!(
            "| {alias} | {skin} | {forms} | {} | {} | {} | {} | {carried} |",
            model(coverage),
            coverage.idle_forms,
            coverage.material_parts,
            coverage.part_copies,
        ));
    }
    lines
}
