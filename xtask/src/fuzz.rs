use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

use dekan_classic::generator::retarget_skin_bin;
use dekan_wad::hash::wad_path_hash;
use dekan_wad::prop::{parse_prop_file, parse_prop_links, serialize_prop_file};
use dekan_wad::wad::{WadArchive, WadFile};

pub struct Rng(u64);

impl Rng {
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }

    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    pub fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound.max(1) as u64) as usize
    }
}

pub struct Corpus {
    pub wads: Vec<Vec<u8>>,
    pub bins: Vec<(String, u32, Vec<u8>)>,
}

fn mini_wad(source: &WadFile, hashes: &[u64]) -> Option<Vec<u8>> {
    let entries: Vec<_> = hashes.iter().filter_map(|h| source.entry(*h)).collect();
    let mut wad = vec![0u8; 272 + 32 * entries.len()];
    wad[0..4].copy_from_slice(b"RW\x03\x04");
    wad[268..272].copy_from_slice(&u32::try_from(entries.len()).ok()?.to_le_bytes());
    for (i, entry) in entries.iter().enumerate() {
        let raw = source.read_raw(entry).ok()?;
        let offset = u32::try_from(wad.len()).ok()?;
        let toc = 272 + 32 * i;
        wad[toc..toc + 8].copy_from_slice(&entry.path_hash.to_le_bytes());
        wad[toc + 8..toc + 12].copy_from_slice(&offset.to_le_bytes());
        wad[toc + 12..toc + 16]
            .copy_from_slice(&u32::try_from(entry.compressed_size).ok()?.to_le_bytes());
        wad[toc + 16..toc + 20]
            .copy_from_slice(&u32::try_from(entry.uncompressed_size).ok()?.to_le_bytes());
        wad[toc + 20] = entry.compression as u8 | (entry.subchunk_count << 4);
        wad[toc + 24..toc + 32].copy_from_slice(&entry.checksum.to_le_bytes());
        wad.extend_from_slice(&raw);
    }
    Some(wad)
}

pub fn corpus(game: &Path, champions: &[(&str, u32)]) -> Corpus {
    let mut corpus = Corpus {
        wads: Vec::new(),
        bins: Vec::new(),
    };
    for (alias, skin) in champions {
        let path = game
            .join("DATA")
            .join("FINAL")
            .join("Champions")
            .join(format!("{alias}.wad.client"));
        let Ok(wad) = WadFile::open(&path) else {
            continue;
        };
        let lower = alias.to_ascii_lowercase();
        let bin = format!("data/characters/{lower}/skins/skin{skin}.bin");
        let skin0 = format!("data/characters/{lower}/skins/skin0.bin");
        let small: Vec<u64> = wad
            .toc()
            .filter(|e| e.compressed_size < 64 * 1024)
            .take(6)
            .map(|e| e.path_hash)
            .collect();
        let mut picked = vec![wad_path_hash(&bin), wad_path_hash(&skin0)];
        picked.extend(small);
        if let Some(mini) = mini_wad(&wad, &picked) {
            corpus.wads.push(mini);
        }
        if let Ok(Some(bytes)) = wad.read(wad_path_hash(&bin)) {
            corpus.bins.push(((*alias).to_owned(), *skin, bytes));
        }
    }
    corpus
}

pub fn mutate(rng: &mut Rng, input: &[u8]) -> Vec<u8> {
    let mut out = input.to_vec();
    let rounds = 1 + rng.below(8);
    for _ in 0..rounds {
        if out.is_empty() {
            out.push(rng.next() as u8);
            continue;
        }
        match rng.below(7) {
            0 => {
                let i = rng.below(out.len());
                out[i] ^= 1 << rng.below(8);
            }
            1 => {
                let i = rng.below(out.len());
                out[i] = [0x00, 0xFF, 0x7F, 0x80, 0x01][rng.below(5)];
            }
            2 => {
                let i = rng.below(out.len().saturating_sub(3).max(1));
                let value: u32 = [0, u32::MAX, 0x7FFF_FFFF, 0x8000_0000, 0xFFFF][rng.below(5)];
                for (k, b) in value.to_le_bytes().iter().enumerate() {
                    if let Some(slot) = out.get_mut(i + k) {
                        *slot = *b;
                    }
                }
            }
            3 => out.truncate(rng.below(out.len())),
            4 => {
                let extra = rng.below(64);
                for _ in 0..extra {
                    out.push(rng.next() as u8);
                }
            }
            5 => {
                let a = rng.below(out.len());
                let b = (a + rng.below(256)).min(out.len());
                let chunk = out[a..b].to_vec();
                let at = rng.below(out.len());
                out.splice(at..at, chunk);
            }
            _ => {
                let a = rng.below(out.len());
                let b = (a + rng.below(64)).min(out.len());
                out.drain(a..b);
            }
        }
    }
    out
}

#[derive(Debug, Default)]
pub struct Report {
    pub iterations: usize,
    pub accepted: usize,
    pub rejected: usize,
    pub panics: Vec<String>,
    pub property_checks: usize,
    pub property_failures: Vec<String>,
}

fn exercise_wad(bytes: &[u8]) -> bool {
    match WadArchive::parse(bytes) {
        Ok(archive) => {
            for entry in archive.entries() {
                let _ = archive.read_entry(entry); // ignore-ok: only a panic matters to the fuzzer
            }
            true
        }
        Err(_) => false,
    }
}

fn exercise_bin(bytes: &[u8], alias: &str, skin: u32) -> bool {
    let links = parse_prop_links(bytes).is_ok();
    let parsed = parse_prop_file(bytes)
        .ok()
        .and_then(|file| serialize_prop_file(&file).ok())
        .is_some();
    let retargeted = retarget_skin_bin(bytes, alias, skin, 0, None).is_ok();
    links || parsed || retargeted
}

fn remember(input: &[u8], kind: &str, alias: &str, skin: u32) {
    let dir = std::env::temp_dir().join("dekan_fuzz_last");
    if std::fs::create_dir_all(&dir).is_ok() {
        let _ = std::fs::write(dir.join("input.bin"), input); // ignore-ok: only a crash reproduction aid
        let _ = std::fs::write(dir.join("kind.txt"), format!("{kind} {alias} {skin}")); // ignore-ok: only a crash reproduction aid
    }
}

pub fn run(corpus: &Corpus, iterations: usize, seed: u64) -> Report {
    let mut report = Report::default();
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));

    for (alias, skin, bytes) in &corpus.bins {
        report.property_checks += 1;
        match parse_prop_file(bytes) {
            Ok(first) => {
                let again = serialize_prop_file(&first)
                    .map_err(|e| e.to_string())
                    .and_then(|out| parse_prop_file(&out).map_err(|e| e.to_string()));
                match again {
                    Ok(second)
                        if second.links == first.links
                            && second.entries.len() == first.entries.len()
                            && second.entries.iter().zip(&first.entries).all(|(a, b)| {
                                a.key_hash == b.key_hash
                                    && a.class_hash == b.class_hash
                                    && a.body == b.body
                            }) => {}
                    Ok(_) => report.property_failures.push(format!(
                        "{alias} skin{skin}: parse(serialize(parse(x))) differs from parse(x)"
                    )),
                    Err(e) => report
                        .property_failures
                        .push(format!("{alias} skin{skin}: re-parse failed: {e}")),
                }
            }
            Err(e) => report
                .property_failures
                .push(format!("{alias} skin{skin}: real bin does not parse: {e}")),
        }
        report.property_checks += 1;
        let target = format!("Characters/{alias}/Skins/Skin0");
        let ok = retarget_skin_bin(bytes, alias, *skin, 0, None)
            .ok()
            .and_then(|out| parse_prop_file(&out).ok())
            .is_some_and(|file| {
                file.entries
                    .iter()
                    .any(|e| e.key_hash == dekan_wad::hash::prop_key_hash(&target))
            });
        if !ok {
            report
                .property_failures
                .push(format!("{alias} skin{skin}: retargeted bin lacks {target}"));
        }
    }

    let mut rng = Rng::new(seed);
    for i in 0..iterations {
        report.iterations += 1;
        let pick_wad = !corpus.wads.is_empty() && (corpus.bins.is_empty() || rng.below(2) == 0);
        let outcome = if pick_wad {
            let source = &corpus.wads[rng.below(corpus.wads.len())];
            let input = mutate(&mut rng, source);
            remember(&input, "wad", "", 0);
            catch_unwind(AssertUnwindSafe(|| exercise_wad(&input)))
                .map_err(|_| format!("WAD iteration {i} (seed {seed})"))
        } else if !corpus.bins.is_empty() {
            let (alias, skin, source) = &corpus.bins[rng.below(corpus.bins.len())];
            let input = mutate(&mut rng, source);
            remember(&input, "bin", alias, *skin);
            catch_unwind(AssertUnwindSafe(|| exercise_bin(&input, alias, *skin)))
                .map_err(|_| format!("PROP iteration {i} (seed {seed}, {alias} skin{skin})"))
        } else {
            break;
        };
        match outcome {
            Ok(true) => report.accepted += 1,
            Ok(false) => report.rejected += 1,
            Err(what) => report.panics.push(what),
        }
    }
    std::panic::set_hook(previous);
    report
}
