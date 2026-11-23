use std::path::PathBuf;
use std::time::Instant;

use dekan_wad::hash::{prop_key_hash, wad_path_hash};
use dekan_wad::prop::{
    PropEntry, PropFile, parse_prop_file, parse_prop_header, parse_prop_links, serialize_prop_file,
};
use dekan_wad::wad::WadFile;
use dekan_wad::writer::{WadWriter, optimal_raw};

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "dekan_wad_fuzz_{tag}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        Self(dir)
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0); // ignore-ok: best-effort test cleanup
    }
}

struct XorShift64(u64);

impl XorShift64 {
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn next_bytes(&mut self, len: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(len);
        for _ in 0..len {
            out.push((self.next_u64() & 0xFF) as u8);
        }
        out
    }
}

#[test]
fn property_wad_roundtrip_preserves_every_payload_verbatim() {
    let mut rng = XorShift64(0x9E37_79B9_7F4A_7C15);
    let dir = TempDir::new("roundtrip");

    for case in 0..25u32 {
        let count = ((rng.next_u64() % 24) + 1) as usize;
        let mut entries = Vec::with_capacity(count);
        let mut writer = WadWriter::default();
        for i in 0..count {
            let path = format!("data/test/case_{case}/entry_{i}.bin");
            let size = (rng.next_u64() % 2048) as usize;
            let data = if i % 3 == 0 {
                vec![(i as u8).wrapping_mul(17); size]
            } else {
                rng.next_bytes(size)
            };
            let hash = wad_path_hash(&path);
            writer.insert(hash, optimal_raw(data.clone()).expect("encode"));
            entries.push((hash, data));
        }

        let wad_path = dir.path().join(format!("case_{case}.wad.client"));
        writer
            .write_to_file(&wad_path, &|| false)
            .expect("write wad");

        let opened = WadFile::open(&wad_path).expect("open wad");
        for (hash, expected) in &entries {
            let actual = opened.read(*hash).expect("read ok").expect("entry present");
            assert_eq!(&actual, expected);
        }
    }
}

#[test]
fn fuzz_malformed_wad_and_prop_buffers_never_panic() {
    let mut rng = XorShift64(0xD1B5_4A32_D192_ED03);
    let dir = TempDir::new("fuzz");
    let scratch = dir.path().join("fuzz.wad.client");

    let valid_prop = serialize_prop_file(&PropFile {
        version: 2,
        links: vec!["characters/zed/skins/skin0.bin".into()],
        entries: vec![PropEntry {
            class_hash: prop_key_hash("SkinCharacterDataProperties"),
            key_hash: prop_key_hash("Characters/Zed/Skins/Skin0"),
            body: vec![0, 0, 0, 0],
        }],
    })
    .expect("valid prop");

    for iter in 0..1000u32 {
        let mut buf = if iter % 4 == 0 {
            let mut copy = valid_prop.clone();
            let flips = ((rng.next_u64() % 6) + 1) as usize;
            for _ in 0..flips {
                let idx = (rng.next_u64() as usize) % copy.len().max(1);
                copy[idx] ^= ((rng.next_u64() % 255) + 1) as u8;
            }
            if iter % 8 == 0 && copy.len() > 2 {
                copy.truncate((rng.next_u64() as usize) % copy.len());
            }
            copy
        } else {
            let len = (rng.next_u64() % 512) as usize;
            rng.next_bytes(len)
        };

        let _ = parse_prop_header(&buf); // ignore-ok: fuzz test asserts no panic
        let _ = parse_prop_links(&buf); // ignore-ok: fuzz test asserts no panic
        let _ = parse_prop_file(&buf); // ignore-ok: fuzz test asserts no panic

        if iter % 5 == 0 {
            if buf.len() >= 3 && iter % 10 == 0 {
                buf[0] = b'R';
                buf[1] = b'W';
                buf[2] = 3;
            }
            std::fs::write(&scratch, &buf).expect("write scratch");
            if let Ok(wad) = WadFile::open(&scratch) {
                let _ = wad.read(rng.next_u64()); // ignore-ok: fuzz test asserts no panic
            }
        }
    }
}

#[test]
fn performance_256_entry_wad_write_and_read_stays_fast() {
    let dir = TempDir::new("perf");
    let wad_path = dir.path().join("perf.wad.client");
    let mut writer = WadWriter::default();
    let mut hashes = Vec::with_capacity(256);
    for i in 0..256u32 {
        let hash = wad_path_hash(&format!("data/perf/item_{i}.bin"));
        writer.insert(
            hash,
            optimal_raw(vec![(i % 251) as u8; 1024]).expect("encode"),
        );
        hashes.push(hash);
    }

    let start = Instant::now();
    writer
        .write_to_file(&wad_path, &|| false)
        .expect("write perf wad");
    let wad = WadFile::open(&wad_path).expect("open perf wad");
    for hash in hashes {
        let got = wad.read(hash).expect("read").expect("present");
        assert_eq!(got.len(), 1024);
    }
    assert!(
        start.elapsed().as_millis() < 2000,
        "256-entry WAD roundtrip took {:?}",
        start.elapsed()
    );
}
