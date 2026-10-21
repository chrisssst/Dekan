use std::collections::{BTreeMap, BTreeSet};

use super::wad::WadReport;

pub(super) const UNKNOWN_FORMAT: &str = "formato de asset que o jogo nao usa mais";

pub(super) fn break_classes(report: &WadReport) -> Vec<&'static str> {
    [
        (
            "bin linkado pendurado antes do reparo",
            report.dangling_before > 0,
        ),
        (
            "bin linkado pendurado depois do reparo",
            !report.dangling_after.is_empty(),
        ),
        ("bin linkado ausente", report.linked_bins_missing > 0),
        (
            "objeto referenciado sem destino",
            !report.refs_only_in_mod.is_empty(),
        ),
        (
            "arquivo referenciado sem destino",
            !report.files_missing.is_empty(),
        ),
        (
            "caminho de asset trocado pelo reparo",
            !report.asset_relinks.is_empty(),
        ),
    ]
    .into_iter()
    .filter_map(|(class, present)| present.then_some(class))
    .collect()
}

pub(super) fn rank(per_mod: &[BTreeSet<&'static str>]) -> Vec<(&'static str, usize)> {
    let mut counts: BTreeMap<&'static str, usize> = BTreeMap::new();
    for class in per_mod.iter().flatten() {
        *counts.entry(class).or_default() += 1;
    }
    let mut ranked: Vec<(&'static str, usize)> = counts.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    ranked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clean_report_has_no_class_and_each_finding_maps_to_its_own() {
        assert!(break_classes(&WadReport::default()).is_empty());

        let report = WadReport {
            dangling_before: 3,
            dangling_after: vec!["data/characters/ahri/skins/skin3.bin".into()],
            refs_only_in_mod: BTreeSet::from([0xdead_beef]),
            ..WadReport::default()
        };
        assert_eq!(
            break_classes(&report),
            [
                "bin linkado pendurado antes do reparo",
                "bin linkado pendurado depois do reparo",
                "objeto referenciado sem destino",
            ]
        );

        let repaired = WadReport {
            dangling_before: 1,
            linked_bins_missing: 2,
            files_missing: BTreeMap::from([(7, "assets/x.dds".into())]),
            asset_relinks: vec![("assets/x.dds".into(), "assets/x.tex".into())],
            ..WadReport::default()
        };
        assert_eq!(
            break_classes(&repaired),
            [
                "bin linkado pendurado antes do reparo",
                "bin linkado ausente",
                "arquivo referenciado sem destino",
                "caminho de asset trocado pelo reparo",
            ]
        );
    }

    #[test]
    fn a_mod_counts_once_per_class_and_the_most_frequent_class_comes_first() {
        let per_mod = [
            BTreeSet::from(["bin linkado ausente", UNKNOWN_FORMAT]),
            BTreeSet::from([UNKNOWN_FORMAT]),
            BTreeSet::from([UNKNOWN_FORMAT, "arquivo referenciado sem destino"]),
            BTreeSet::new(),
        ];
        assert_eq!(
            rank(&per_mod),
            [
                (UNKNOWN_FORMAT, 3),
                ("arquivo referenciado sem destino", 1),
                ("bin linkado ausente", 1),
            ]
        );
        assert!(rank(&[]).is_empty());
    }
}
