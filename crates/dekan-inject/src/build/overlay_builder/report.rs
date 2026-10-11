use super::*;

pub(super) fn log_shared_copies(
    game: &BTreeMap<String, GameWad>,
    mounts: &MountRoles,
    mods: &[String],
) {
    let (mut count, mut bytes, mut size_unknown) = (0usize, 0u64, 0usize);
    for name in mounts.shared_only() {
        count += 1;
        match game.get(name).map(|wad| std::fs::metadata(&wad.path)) {
            Some(Ok(meta)) => bytes += meta.len(),
            _ => size_unknown += 1,
        }
    }
    if count == 0 {
        return;
    }
    info!(
        mods = ?mods,
        shared_wads = count,
        shared_bytes = bytes,
        size_unknown,
        "Copying additional game WADs whole because they share entries with the mods"
    );
}

pub(super) fn header_hex(path: &Path) -> Option<String> {
    use std::io::Read;
    let mut head = [0u8; 272];
    std::fs::File::open(path).ok()?.read_exact(&mut head).ok()?;
    Some(head.iter().map(|b| format!("{b:02x}")).collect())
}

pub(super) fn log_overlay_wad(
    game_wad: Option<&GameWad>,
    mount: &str,
    wad: &OverlayWad,
    outcome: WriteOutcome,
    mode: &str,
    out: &Path,
) -> serde_json::Value {
    let game_toc = game_wad.and_then(|g| WadFile::open_toc_only(&g.path).ok());
    let mut entries = Vec::new();
    let (mut replaced, mut added) = (0usize, 0usize);
    for (hash, entry) in wad.writer.inserted() {
        let original = game_toc.as_ref().and_then(|t| t.entry(hash));
        if original.is_some() {
            replaced += 1;
        } else {
            added += 1;
        }
        let record = serde_json::json!({
            "path_hash": format!("{hash:016x}"),
            "change": if original.is_some() { "replaced" } else { "added" },
            "stored_bytes": WadWriter::stored_len_of(entry),
            "decoded_bytes": entry.uncompressed_size,
            "kind": entry.kind,
            "checksum": format!("{:016x}", entry.checksum),
            "game_stored_bytes": original.map(|o| o.compressed_size),
            "game_decoded_bytes": original.map(|o| o.uncompressed_size),
            "game_kind": original.map(|o| o.compression as u8),
            "game_checksum": original.map(|o| format!("{:016x}", o.checksum)),
        });
        debug!(mount, entry = %record, "Overlay entry");
        entries.push(record);
    }
    let game_header = game_wad.and_then(|g| header_hex(&g.path));
    let overlay_header = header_hex(out);
    let mode = if matches!(outcome, WriteOutcome::Unchanged { .. }) {
        "unchanged since last build"
    } else {
        mode
    };
    info!(
        mount,
        file = %wad.relpath.display(),
        map = game_wad.is_some_and(is_map),
        mode,
        entries = wad.writer.len(),
        replaced,
        added,
        header_matches_game = game_header.is_some() && game_header == overlay_header,
        bytes = outcome.bytes(),
        "Overlay WAD written"
    );
    serde_json::json!({
        "mount": mount,
        "file": wad.relpath.display().to_string(),
        "map": game_wad.is_some_and(is_map),
        "mode": mode,
        "entries": wad.writer.len(),
        "replaced": replaced,
        "added": added,
        "bytes": outcome.bytes(),
        "game_header": game_header,
        "overlay_header": overlay_header,
        "changes": entries,
    })
}

pub(super) fn write_overlay_manifest(
    overlay_dir: &Path,
    mods: &[String],
    wads: &[serde_json::Value],
) {
    let Some(parent) = overlay_dir.parent() else {
        return;
    };
    let manifest = serde_json::json!({
        "builder_revision": OVERLAY_BUILDER_REVISION,
        "mods": mods,
        "wads": wads,
    });
    let path = parent.join("overlay_manifest.json");
    match serde_json::to_vec_pretty(&manifest) {
        Ok(bytes) => {
            if let Err(e) = std::fs::write(&path, bytes) {
                warn!(file = %path.display(), error = %e, "Overlay manifest not written");
            }
        }
        Err(e) => warn!(error = %e, "Overlay manifest not serialized"),
    }
}
