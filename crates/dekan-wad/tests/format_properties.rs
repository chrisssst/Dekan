use std::io::Cursor;

use dekan_wad::hash::content_checksum;
use dekan_wad::modpkg::{MAGIC, ModPkg};
use dekan_wad::prop::tree::{FIELD_FILE, Field, FieldShapes, Value, parse_fields, write_fields};
use dekan_wad::wad::WadArchive;
use dekan_wad::writer::{WadWriter, optimal_raw};

const STRING: u8 = 16;
const FIXED: [(u8, usize); 20] = [
    (0, 0),
    (1, 1),
    (2, 1),
    (3, 1),
    (4, 2),
    (5, 2),
    (6, 4),
    (7, 4),
    (8, 8),
    (9, 8),
    (10, 4),
    (11, 8),
    (12, 12),
    (13, 16),
    (14, 64),
    (15, 4),
    (17, 4),
    (18, 8),
    (0x84, 4),
    (0x87, 1),
];

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }

    fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| self.next() as u8).collect()
    }

    fn text(&mut self, max: u64) -> String {
        const ALPHABET: &[u8] = b"abcdefXYZ_/.0123456789";
        (0..self.below(max) + 1)
            .map(|_| ALPHABET[self.below(ALPHABET.len() as u64) as usize] as char)
            .collect()
    }
}

fn encoded_string(text: &str) -> Vec<u8> {
    let mut bytes = u16::try_from(text.len())
        .expect("short")
        .to_le_bytes()
        .to_vec();
    bytes.extend_from_slice(text.as_bytes());
    bytes
}

fn primitive(rng: &mut Rng, kind: u8) -> Value {
    let bytes = if kind == STRING {
        encoded_string(&rng.text(40))
    } else {
        let size = FIXED
            .iter()
            .find(|(k, _)| *k == kind)
            .expect("fixed kind")
            .1;
        rng.bytes(size)
    };
    Value::Raw { kind, bytes }
}

fn primitive_kind(rng: &mut Rng) -> u8 {
    if rng.chance(25) {
        STRING
    } else {
        FIXED[rng.below(FIXED.len() as u64) as usize].0
    }
}

fn fields(rng: &mut Rng, depth: u32) -> Vec<Field> {
    (0..rng.below(6))
        .map(|_| Field {
            name: rng.next() as u32,
            value: value(rng, depth),
        })
        .collect()
}

fn structure(rng: &mut Rng, depth: u32) -> Value {
    let kind = if rng.chance(50) { 0x82 } else { 0x83 };
    if rng.chance(10) {
        return Value::Struct {
            kind,
            class: 0,
            fields: Vec::new(),
        };
    }
    Value::Struct {
        kind,
        class: (rng.next() as u32).max(1),
        fields: fields(rng, depth + 1),
    }
}

fn value(rng: &mut Rng, depth: u32) -> Value {
    if depth >= 4 || rng.chance(55) {
        let kind = primitive_kind(rng);
        return primitive(rng, kind);
    }
    match rng.below(4) {
        0 => {
            let element = if rng.chance(30) {
                0x83
            } else {
                primitive_kind(rng)
            };
            let items = (0..rng.below(5))
                .map(|_| {
                    if element == 0x83 {
                        Value::Struct {
                            kind: 0x83,
                            class: (rng.next() as u32).max(1),
                            fields: fields(rng, depth + 1),
                        }
                    } else {
                        primitive(rng, element)
                    }
                })
                .collect();
            Value::List {
                kind: if rng.chance(50) { 0x80 } else { 0x81 },
                element,
                items,
            }
        }
        1 => structure(rng, depth),
        2 => {
            let inner = primitive_kind(rng);
            Value::Optional {
                inner,
                value: rng.chance(60).then(|| Box::new(primitive(rng, inner))),
            }
        }
        _ => {
            let key = [7u8, 17, STRING][rng.below(3) as usize];
            let value_kind = primitive_kind(rng);
            let entries = (0..rng.below(5))
                .map(|_| (primitive(rng, key), primitive(rng, value_kind)))
                .collect();
            Value::Map {
                key,
                value: value_kind,
                entries,
            }
        }
    }
}

#[test]
fn hundreds_of_wad_entries_come_back_byte_for_byte_with_their_checksums() {
    let mut rng = Rng(0x5EED_0001);
    let mut writer = WadWriter::default();
    let mut expected = Vec::new();
    for _ in 0..400 {
        let hash = rng.next();
        let size = rng.below(48 * 1024) as usize;
        let mut content = if rng.chance(50) {
            vec![rng.next() as u8; size]
        } else {
            rng.bytes(size)
        };
        if rng.chance(10) {
            content.splice(0..0, *b"BKHD");
        }
        writer.insert(hash, optimal_raw(content.clone()).expect("entry"));
        expected.retain(|(h, _)| *h != hash);
        expected.push((hash, content));
    }
    let bytes = writer.to_bytes().expect("wad");
    let wad = WadArchive::parse(&bytes).expect("parse");

    assert_eq!(wad.entries().len(), expected.len());
    assert!(
        wad.entries()
            .windows(2)
            .all(|w| w[0].path_hash < w[1].path_hash),
        "the table of contents is sorted by path hash"
    );
    for (hash, content) in &expected {
        let entry = wad.find_by_hash(*hash).expect("entry present");
        assert_eq!(
            &wad.read_entry(entry).expect("read"),
            content,
            "{hash:016x}"
        );
        let stored = wad.raw_payload(entry).expect("payload");
        assert_eq!(entry.checksum, content_checksum(stored), "{hash:016x}");
    }
}

#[test]
fn thousands_of_damaged_wads_are_refused_or_read_without_a_panic() {
    let mut rng = Rng(0x5EED_0002);
    let mut writer = WadWriter::default();
    for _ in 0..24 {
        let size = rng.below(4096) as usize;
        let content = vec![rng.next() as u8; size];
        writer.insert(rng.next(), optimal_raw(content).expect("entry"));
    }
    let valid = writer.to_bytes().expect("wad");

    for round in 0..4000 {
        let mut damaged = valid.clone();
        if round % 4 == 0 {
            damaged.truncate(rng.below(valid.len() as u64) as usize);
        } else {
            for _ in 0..rng.below(8) + 1 {
                let at = rng.below(damaged.len() as u64) as usize;
                damaged[at] = rng.next() as u8;
            }
        }
        if let Ok(wad) = WadArchive::parse(&damaged) {
            for entry in wad.entries() {
                let _ = wad.read_entry(entry); // ignore-ok: only the absence of a panic or an abort is under test
            }
        }
    }
}

#[test]
fn random_property_trees_round_trip_and_every_truncation_is_refused() {
    let mut rng = Rng(0x5EED_0003);
    for _ in 0..600 {
        let tree = fields(&mut rng, 0);
        let bytes = write_fields(&tree).expect("write");
        let parsed = parse_fields(&bytes).expect("parse");
        assert_eq!(parsed, tree);
        assert_eq!(write_fields(&parsed).expect("rewrite"), bytes);
        for cut in 0..bytes.len() {
            assert!(
                parse_fields(&bytes[..cut]).is_err(),
                "a body cut at {cut} of {} bytes was accepted",
                bytes.len()
            );
        }
        let mut damaged = bytes.clone();
        for _ in 0..4 {
            if damaged.is_empty() {
                break;
            }
            let at = rng.below(damaged.len() as u64) as usize;
            damaged[at] = rng.next() as u8;
            if let Ok(fields) = parse_fields(&damaged) {
                let _ = write_fields(&fields); // ignore-ok: only the absence of a panic is under test
            }
        }
    }
}

fn game_and_stale(
    rng: &mut Rng,
    depth: u32,
    names: &mut u32,
) -> (Vec<Field>, Vec<Field>, Vec<Field>) {
    let (mut game, mut stale, mut fixed) = (Vec::new(), Vec::new(), Vec::new());
    for _ in 0..rng.below(6) {
        *names += 1;
        let name = *names;
        let path = rng.text(30);
        let file = Value::Raw {
            kind: FIELD_FILE,
            bytes: xxhash_rust::xxh64::xxh64(path.to_ascii_lowercase().as_bytes(), 0)
                .to_le_bytes()
                .to_vec(),
        };
        let text = Value::Raw {
            kind: STRING,
            bytes: encoded_string(&path),
        };
        let (g, s, f) = match rng.below(5) {
            0 => (primitive(rng, FIELD_FILE), text, file),
            1 => (
                Value::Optional {
                    inner: FIELD_FILE,
                    value: None,
                },
                Value::Optional {
                    inner: STRING,
                    value: Some(Box::new(text)),
                },
                Value::Optional {
                    inner: FIELD_FILE,
                    value: Some(Box::new(file)),
                },
            ),
            2 => (
                Value::List {
                    kind: 0x80,
                    element: FIELD_FILE,
                    items: vec![],
                },
                Value::List {
                    kind: 0x80,
                    element: STRING,
                    items: vec![text.clone(), text],
                },
                Value::List {
                    kind: 0x80,
                    element: FIELD_FILE,
                    items: vec![file.clone(), file],
                },
            ),
            3 if depth < 3 => {
                let class = (rng.next() as u32).max(1);
                let (g, s, f) = game_and_stale(rng, depth + 1, names);
                let wrap = |fields| Value::Struct {
                    kind: 0x83,
                    class,
                    fields,
                };
                (wrap(g), wrap(s), wrap(f))
            }
            _ => {
                let same = primitive(rng, STRING);
                (same.clone(), same.clone(), same)
            }
        };
        game.push(Field { name, value: g });
        stale.push(Field { name, value: s });
        fixed.push(Field { name, value: f });
    }
    (game, stale, fixed)
}

#[test]
fn retyping_converts_exactly_the_fields_the_game_declares_as_files() {
    let mut rng = Rng(0x5EED_0004);
    for _ in 0..800 {
        let mut names = 0;
        let (game, mut stale, fixed) = game_and_stale(&mut rng, 0, &mut names);
        let mut shapes = FieldShapes::default();
        shapes.record(0xC1A5, &game);
        shapes.strings_to_files(0xC1A5, &mut stale);
        assert_eq!(stale, fixed);
        assert_eq!(shapes.strings_to_files(0xC1A5, &mut stale), 0, "idempotent");
        assert!(write_fields(&stale).is_ok());
    }
}

fn package(rng: &mut Rng, chunks: usize) -> Vec<u8> {
    let mut out = MAGIC.to_vec();
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&u32::try_from(chunks).expect("count").to_le_bytes());
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&4u32.to_le_bytes());
    out.extend_from_slice(b"base");
    out.extend_from_slice(&0i32.to_le_bytes());
    out.extend_from_slice(&u32::try_from(chunks).expect("count").to_le_bytes());
    for index in 0..chunks {
        out.extend_from_slice(format!("p{index}.bin\0").as_bytes());
    }
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(b"zed.wad.client\0");
    out.resize(out.len().next_multiple_of(8), 0);
    let contents: Vec<Vec<u8>> = (0..chunks)
        .map(|_| {
            let len = rng.below(300) as usize;
            rng.bytes(len)
        })
        .collect();
    let mut offset = (out.len() + chunks * 61) as u64;
    for (index, content) in contents.iter().enumerate() {
        let checksum = content_checksum(content);
        out.extend_from_slice(&rng.next().to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        out.push(0);
        out.extend_from_slice(&(content.len() as u64).to_le_bytes());
        out.extend_from_slice(&(content.len() as u64).to_le_bytes());
        out.extend_from_slice(&checksum.to_le_bytes());
        out.extend_from_slice(&checksum.to_le_bytes());
        out.extend_from_slice(&u32::try_from(index).expect("index").to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        offset += content.len() as u64;
    }
    for content in contents {
        out.extend_from_slice(&content);
    }
    out
}

fn mount(bytes: &[u8]) -> Result<usize, dekan_wad::error::WadError> {
    let package = ModPkg::read_index(Cursor::new(bytes))?;
    Ok(package
        .base_wads(Cursor::new(bytes))?
        .values()
        .map(|w| w.len())
        .sum())
}

#[test]
fn a_modpkg_cut_anywhere_is_refused_and_damage_never_panics() {
    let mut rng = Rng(0x5EED_0005);
    for _ in 0..40 {
        let chunks = rng.below(6) as usize + 1;
        let bytes = package(&mut rng, chunks);
        assert!(mount(&bytes).expect("whole package") <= chunks);
        for cut in 0..bytes.len() {
            assert!(
                mount(&bytes[..cut]).is_err(),
                "cut at {cut} of {}",
                bytes.len()
            );
        }
        for _ in 0..200 {
            let mut damaged = bytes.clone();
            let at = rng.below(damaged.len() as u64) as usize;
            damaged[at] = rng.next() as u8;
            let _ = mount(&damaged); // ignore-ok: only the absence of a panic is under test
        }
    }
}

#[test]
fn forged_sizes_are_refused_without_allocating_what_they_declare() {
    let mut writer = WadWriter::default();
    writer.insert(7, optimal_raw(vec![1; 64]).expect("entry"));
    let mut bytes = writer.to_bytes().expect("wad");
    let size_field = 272 + 16;
    bytes[size_field..size_field + 4].copy_from_slice(&0x7FFF_FFFFu32.to_le_bytes());
    assert!(matches!(
        WadArchive::parse(&bytes),
        Err(dekan_wad::error::WadError::EntryTooLarge { path_hash: 7, .. })
    ));

    let bomb = zstd::bulk::compress(&vec![0u8; 64 * 1024 * 1024], 19).expect("bomb");
    assert!(bomb.len() < 64 * 1024, "the frame is tiny: {}", bomb.len());
    let started = std::time::Instant::now();
    assert_eq!(dekan_wad::writer::decode_zstd_bounded(&bomb, 100), None);
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
    assert_eq!(
        dekan_wad::writer::decode_zstd_bounded(
            &zstd::bulk::compress(b"exact", 3).expect("frame"),
            5
        ),
        Some(b"exact".to_vec())
    );
}
