use super::*;

pub(crate) fn run_wad_writer_probe(args: &[String]) {
    use dekan_wad::hash::content_checksum;
    use dekan_wad::wad::WadFile;
    use dekan_wad::writer::{WadWriter, WriterEntry};
    use std::time::Instant;

    let mut root = None;
    let mut filters = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--root" {
            root = iter.next().map(PathBuf::from);
        } else {
            filters.push(arg.to_ascii_lowercase());
        }
    }
    if filters.is_empty() {
        println!(
            "uso: cargo xtask wad-writer-probe [--root <pasta>] <filtro...>   (ex.: Champions/Annie, Maps/Shipping; '.' = todos)"
        );
        return;
    }
    let scratch =
        std::env::temp_dir().join(format!("dekan_wad_writer_probe_{}", std::process::id()));
    let final_dir = match root {
        Some(root) => root,
        None => {
            let Some(game) = game_dir() else {
                return;
            };
            game.join("DATA").join("FINAL")
        }
    };
    let mut files = Vec::new();
    collect_wad_files(&final_dir, &mut files);
    files.retain(|p| {
        let rel = p
            .strip_prefix(&final_dir)
            .unwrap_or(p)
            .to_string_lossy()
            .replace('\\', "/")
            .to_ascii_lowercase();
        filters.iter().any(|f| rel.contains(f.as_str()))
    });
    files.sort();
    println!(
        "raiz: {} | WADs selecionados: {}",
        final_dir.display(),
        files.len()
    );

    let (mut total_entries, mut total_bad) = (0usize, 0usize);
    for file in &files {
        let rel = file.strip_prefix(&final_dir).unwrap_or(file);
        let source = match WadFile::open_toc_only(file) {
            Ok(source) => source,
            Err(e) => {
                println!("  {} nao abriu: {e}", rel.display());
                continue;
            }
        };
        let mut writer = WadWriter::rebased_on(&source);
        let index = writer.add_source(file);
        for entry in source.toc() {
            writer.insert(entry.path_hash, WriterEntry::from_wad(index, entry));
        }
        let out = scratch.join(rel);
        let started = Instant::now();
        let outcome = match writer.write_to_file(&out, &|| false) {
            Ok(outcome) => outcome,
            Err(e) => {
                println!("  {} copia falhou: {e}", rel.display());
                continue;
            }
        };
        let elapsed = started.elapsed();

        let copy = match WadFile::open_toc_only(&out) {
            Ok(copy) => copy,
            Err(e) => {
                println!("  [ERRO] {} copia ilegivel: {e}", rel.display());
                total_bad += 1;
                continue;
            }
        };
        let mut bad = 0usize;
        if copy.len() != source.len() || copy.signature() != source.signature() {
            bad += 1;
        }
        for original in source.toc() {
            let same = copy.entry(original.path_hash).is_some_and(|copied| {
                copied.compression == original.compression
                    && copied.compressed_size == original.compressed_size
                    && copied.uncompressed_size == original.uncompressed_size
                    && copied.checksum == original.checksum
                    && copied.subchunk_count == original.subchunk_count
                    && copied.first_subchunk == original.first_subchunk
                    && copy
                        .read_raw(copied)
                        .is_ok_and(|raw| content_checksum(&raw) == original.checksum)
            });
            if !same {
                bad += 1;
            }
        }
        total_entries += source.len();
        total_bad += bad;
        println!(
            "  {:>6} entradas | {:>8.2} MB | copia {:>6} ms | {} divergentes | {}",
            source.len(),
            outcome.bytes() as f64 / 1_048_576.0,
            elapsed.as_millis(),
            bad,
            rel.display()
        );
        let _ = std::fs::remove_file(&out); // ignore-ok: probe scratch copy
    }
    let _ = std::fs::remove_dir_all(&scratch); // ignore-ok: probe scratch folder
    println!("\ntotal: {total_entries} entradas, {total_bad} divergentes");
}
