use crate::game_dir;
use std::path::PathBuf;

pub(crate) fn run_prop_roundtrip(args: &[String]) {
    let root = args
        .iter()
        .position(|a| a == "--root")
        .and_then(|p| args.get(p + 1))
        .map(PathBuf::from);
    let Some(game) = root.or_else(game_dir) else {
        return;
    };
    let final_dir = game.join("DATA").join("FINAL");
    let mut wads: Vec<PathBuf> = ["Champions", "Maps/Shipping"]
        .iter()
        .filter_map(|dir| std::fs::read_dir(final_dir.join(dir)).ok())
        .flat_map(|entries| entries.flatten().map(|e| e.path()))
        .filter(|p| p.to_string_lossy().ends_with(".wad.client"))
        .collect();
    wads.sort();
    let (mut bins, mut objects, mut differs, mut unreadable) = (0usize, 0usize, 0usize, 0usize);
    let mut examples = Vec::new();
    for path in &wads {
        let Ok(wad) = dekan_wad::wad::WadFile::open(path) else {
            unreadable += 1;
            continue;
        };
        let hashes: Vec<u64> = wad.entries().map(|(hash, _)| hash).collect();
        for hash in hashes {
            let Ok(Some(bytes)) = wad.read(hash) else {
                continue;
            };
            if !(bytes.starts_with(b"PROP") || bytes.starts_with(b"PTCH")) {
                continue;
            }
            let Ok(bin) = dekan_wad::prop::parse_prop_file(&bytes) else {
                continue;
            };
            bins += 1;
            for entry in &bin.entries {
                objects += 1;
                let same = dekan_wad::prop::tree::parse_fields(&entry.body)
                    .and_then(|fields| dekan_wad::prop::tree::write_fields(&fields))
                    .is_ok_and(|written| written == entry.body);
                if !same {
                    differs += 1;
                    if examples.len() < 10 {
                        examples.push(format!(
                            "{} entry {hash:016x} object {:08x}",
                            path.file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_default(),
                            entry.key_hash
                        ));
                    }
                }
            }
        }
    }
    println!(
        "wads: {} ({unreadable} unreadable) | bins: {bins} | objects: {objects} | not identical: {differs}",
        wads.len()
    );
    for example in &examples {
        println!("  {example}");
    }
    if differs > 0 {
        std::process::exit(1);
    }
}

#[derive(Default)]
struct WadTypeStats {
    entries: usize,
    decoded: usize,
    failed: usize,
    size_mismatch: usize,
    checksum_mismatch: usize,
    zstd_magic_first: usize,
    gzip_magic_first: usize,
    compressed_bytes: u64,
}

pub(crate) fn run_wad_probe(args: &[String]) {
    use std::collections::BTreeMap;

    use dekan_wad::hash::content_checksum;
    use dekan_wad::wad::WadArchive;

    const ZSTD_MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];
    const GZIP_MAGIC: [u8; 2] = [0x1F, 0x8B];
    const MAX_REPORTED: usize = 10;

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
            "uso: cargo xtask wad-probe [--root <pasta>] <trecho do caminho...>   (ex.: Champions/Annie. Maps/Shipping; '.' = todos)"
        );
        return;
    }
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
        "raiz: {}  WADs selecionados: {}",
        final_dir.display(),
        files.len()
    );

    let mut stats: BTreeMap<u8, WadTypeStats> = BTreeMap::new();
    let mut reported = Vec::new();
    for file in &files {
        let data = match std::fs::read(file) {
            Ok(data) => data,
            Err(e) => {
                println!("  {} nao leu: {e}", file.display());
                continue;
            }
        };
        let mut archive = match WadArchive::parse(&data) {
            Ok(archive) => archive,
            Err(e) => {
                println!("  {} nao abriu: {e}", file.display());
                continue;
            }
        };

        if let Some(name) = dekan_wad::wad::subchunk_toc_name(file) {
            archive.load_subchunk_toc(&name);
        }
        let mut file_problems = 0usize;
        for entry in archive.entries() {
            let kind = entry.compression as u8;
            let s = stats.entry(kind).or_default();
            s.entries += 1;
            s.compressed_bytes += entry.compressed_size as u64;
            let payload = data
                .get(entry.offset..entry.offset + entry.compressed_size)
                .unwrap_or(&[]);
            if payload.starts_with(&ZSTD_MAGIC) {
                s.zstd_magic_first += 1;
            }
            if payload.starts_with(&GZIP_MAGIC) {
                s.gzip_magic_first += 1;
            }
            if content_checksum(payload) != entry.checksum {
                s.checksum_mismatch += 1;
                file_problems += 1;
            }
            match archive.read_entry(entry) {
                Ok(_) => s.decoded += 1,
                Err(e) => {
                    file_problems += 1;
                    if matches!(e, dekan_wad::error::WadError::SizeMismatch { .. }) {
                        s.size_mismatch += 1;
                    } else {
                        s.failed += 1;
                    }
                    if reported.len() < MAX_REPORTED {
                        let head: Vec<String> =
                            payload.iter().take(8).map(|b| format!("{b:02x}")).collect();
                        reported.push(format!(
                            "{} {:016x} tipo {kind}: {e} (inicio {})",
                            file.display(),
                            entry.path_hash,
                            head.join(" ")
                        ));
                    }
                }
            }
        }

        println!(
            "  {:>6} entradas  {:>4} problemas  {}",
            archive.entries().len(),
            file_problems,
            file.strip_prefix(&final_dir).unwrap_or(file).display()
        );
    }

    println!(
        "\ntipo  entradas  decodificadas  falhas  tamanho!=TOC  checksum!=TOC  comeca_zstd  comeca_gzip  MB_comprimidos"
    );
    for (kind, s) in &stats {
        println!(
            "{kind:>4}  {:>8}  {:>13}  {:>6}  {:>12}  {:>13}  {:>11}  {:>11}  {:>14.1}",
            s.entries,
            s.decoded,
            s.failed,
            s.size_mismatch,
            s.checksum_mismatch,
            s.zstd_magic_first,
            s.gzip_magic_first,
            s.compressed_bytes as f64 / 1_048_576.0
        );
    }
    if !reported.is_empty() {
        println!("\nprimeiros problemas:");
        for line in &reported {
            println!("  {line}");
        }
    }
}

pub(crate) fn run_wad_types(args: &[String]) {
    use std::collections::BTreeMap;

    let root = match args.iter().position(|a| a == "--root") {
        Some(i) => match args.get(i + 1) {
            Some(root) => PathBuf::from(root),
            None => {
                println!("uso: cargo xtask wad-types [--root <pasta>]");
                return;
            }
        },
        None => match game_dir() {
            Some(game) => game.join("DATA").join("FINAL"),
            None => return,
        },
    };
    let mut files = Vec::new();
    collect_wad_files(&root, &mut files);
    println!("raiz: {} | WADs: {}", root.display(), files.len());

    let mut by_type: BTreeMap<u8, usize> = BTreeMap::new();
    let mut samples: BTreeMap<u8, Vec<String>> = BTreeMap::new();
    let mut unreadable = 0usize;
    for file in &files {
        let wad = match dekan_wad::wad::WadFile::open(file) {
            Ok(wad) => wad,
            Err(e) => {
                unreadable += 1;
                println!("  nao abriu {}: {e}", file.display());
                continue;
            }
        };
        for entry in wad.toc() {
            let kind = entry.compression as u8;
            *by_type.entry(kind).or_default() += 1;
            if matches!(kind, 1 | 2) && samples.get(&kind).is_none_or(|s| s.len() < 5) {
                let head = match wad.read_raw(entry) {
                    Ok(raw) => raw
                        .iter()
                        .take(24)
                        .map(|b| format!("{b:02X}"))
                        .collect::<Vec<_>>()
                        .join(" "),
                    Err(e) => format!("ilegivel: {e}"),
                };
                samples.entry(kind).or_default().push(format!(
                    "{:016x} em {}: {head}",
                    entry.path_hash,
                    file.strip_prefix(&root).unwrap_or(file).display()
                ));
            }
        }
    }

    println!("\ntipo  entradas   (como o dekan-wad le o numero)");
    for (kind, count) in &by_type {
        let name = dekan_wad::wad::CompressionType::from_type_byte(*kind)
            .map(|c| format!("{c:?}"))
            .unwrap_or_else(|_| "?".into());
        println!("  {kind}   {count:>9}   {name}");
    }
    for (kind, lines) in &samples {
        println!("\namostras do tipo {kind}:");
        for line in lines {
            println!("  {line}");
        }
    }
    if unreadable > 0 {
        println!("\nWADs ilegiveis: {unreadable}");
    }
}

pub(crate) fn collect_wad_files(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in read.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_wad_files(&path, out);
        } else if path.to_string_lossy().ends_with(".wad.client") {
            out.push(path);
        }
    }
}

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
