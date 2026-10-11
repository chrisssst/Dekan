use super::*;

#[derive(Default)]
pub(super) struct WadReport {
    pub(super) changed_entries: Vec<String>,
    pub(super) other_entries_identical: bool,
    pub(super) relinked: Vec<(String, String)>,
    pub(super) asset_relinks: Vec<(String, String)>,
    pub(super) dangling_before: usize,
    pub(super) dangling_after: Vec<String>,
    pub(super) same_as_game: usize,
    pub(super) differs_from_game: usize,
    pub(super) not_in_game: usize,
    pub(super) props: usize,
    pub(super) refs_only_in_mod: BTreeSet<u32>,
    pub(super) linked_bins_missing: usize,
    pub(super) files_checked: BTreeSet<u64>,
    pub(super) files_missing: BTreeMap<u64, String>,
}

pub(super) fn audit_wad(
    before: &ModWad,
    after: &ModWad,
    own_before: &HashSet<u64>,
    own_after: &HashSet<u64>,
    game: &mut Game,
) -> Result<WadReport, String> {
    let old = WadFile::open(&before.readable).map_err(|e| e.to_string())?;
    let new = WadFile::open(&after.readable).map_err(|e| e.to_string())?;
    let mut report = WadReport {
        other_entries_identical: true,
        dangling_before: check(&before.source, &game.hashes, own_before)
            .map_err(|e| e.to_string())?
            .dangling
            .len(),
        dangling_after: check(&after.source, &game.hashes, own_after)
            .map_err(|e| e.to_string())?
            .dangling,
        ..WadReport::default()
    };

    let old_hashes: BTreeSet<u64> = old.toc().map(|e| e.path_hash).collect();
    let new_hashes: BTreeSet<u64> = new.toc().map(|e| e.path_hash).collect();
    report.other_entries_identical &= old_hashes == new_hashes;

    let mut game_files = game.hashes.clone();
    game_files.extend(own_after);
    let mut mod_keys: HashSet<u32> = HashSet::new();
    let mut prop_hashes = BTreeSet::new();
    for hash in &new_hashes {
        if let Some(prop) = new
            .read(*hash)
            .ok()
            .flatten()
            .and_then(|b| parse_prop_file(&b).ok())
        {
            mod_keys.extend(prop.entries.iter().map(|e| e.key_hash));
            prop_hashes.insert(*hash);
        }
    }
    let mut archive_keys = game.keys_of_archives_holding(&prop_hashes);
    archive_keys.extend(game.global_keys());

    for hash in &new_hashes {
        let after_bytes = new
            .read(*hash)
            .map_err(|e| e.to_string())?
            .unwrap_or_default();
        let before_bytes = old
            .read(*hash)
            .map_err(|e| e.to_string())?
            .unwrap_or_default();
        if after_bytes != before_bytes {
            report.changed_entries.push(format!("{hash:016x}"));
            match (
                parse_prop_file(&before_bytes),
                parse_prop_file(&after_bytes),
            ) {
                (Ok(a), Ok(b))
                    if a.version == b.version
                        && a.links.len() == b.links.len()
                        && a.entries.len() == b.entries.len() =>
                {
                    report.relinked.extend(
                        a.links
                            .iter()
                            .zip(&b.links)
                            .filter(|(x, y)| x != y)
                            .map(|(x, y)| (x.clone(), y.clone())),
                    );
                    for (x, y) in a.entries.iter().zip(&b.entries) {
                        if x.class_hash != y.class_hash || x.key_hash != y.key_hash {
                            report.other_entries_identical = false;
                            continue;
                        }
                        if x.body == y.body {
                            continue;
                        }
                        report.other_entries_identical &=
                            match (tree::parse_fields(&x.body), tree::parse_fields(&y.body)) {
                                (Ok(fx), Ok(fy)) => {
                                    same_but_asset_paths(&fx, &fy, &mut report.asset_relinks)
                                }
                                _ => false,
                            };
                    }
                }
                _ => report.other_entries_identical = false,
            }
        }

        match game.read(*hash) {
            Some(game_bytes) if game_bytes == after_bytes => report.same_as_game += 1,
            Some(_) => report.differs_from_game += 1,
            None => report.not_in_game += 1,
        }

        let Ok(prop) = parse_prop_file(&after_bytes) else {
            continue;
        };
        report.props += 1;
        let (mut keys, missing) = closure_keys(&prop, &new, game);
        report.linked_bins_missing += missing;
        keys.extend(&mod_keys);
        keys.extend(&archive_keys);
        let mut unresolved = unresolved_refs(&prop, &keys);
        if let Some(baseline) = game.read(*hash).and_then(|b| parse_prop_file(&b).ok()) {
            let (game_keys, _) = closure_keys(&baseline, &new, game);
            let expected = unresolved_refs(&baseline, &game_keys);
            unresolved.retain(|r| !expected.contains(r));
        }
        report.refs_only_in_mod.extend(unresolved);

        let baseline_files: HashSet<u64> = game
            .read(*hash)
            .and_then(|b| parse_prop_file(&b).ok())
            .map(|p| prop_files(&p).into_iter().map(|(_, h)| h).collect())
            .unwrap_or_default();
        for (name, file) in prop_files(&prop) {
            report.files_checked.insert(file);
            if !game_files.contains(&file) && !baseline_files.contains(&file) {
                report.files_missing.entry(file).or_insert(name);
            }
        }
    }
    Ok(report)
}
