use super::*;
use crate::wad::{WadArchive, WadFile};

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "dekan_writer_{name}_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: fixture may not exist yet
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn memory(kind: u8, bytes: &[u8], uncompressed: u64) -> WriterEntry {
    WriterEntry {
        kind,
        subchunk_count: 0,
        first_subchunk: 0,
        uncompressed_size: uncompressed,
        checksum: content_checksum(bytes),
        payload: Payload::Memory(Arc::from(bytes.to_vec())),
    }
}

#[test]
fn test_header_follows_cslol_and_toc_is_sorted_and_deduplicated() {
    let mut signature = [0u8; WAD_SIGNATURE_SIZE];
    signature[0] = 0xAB;
    signature[255] = 0xCD;
    let mut writer = WadWriter::new(signature);
    writer.insert(0x30, memory(0, b"shared", 6));
    writer.insert(0x10, memory(0, b"shared", 6));
    writer.insert(0x20, memory(0, b"other", 5));
    let bytes = writer.to_bytes().expect("write");

    assert_eq!(&bytes[0..4], &VERSION);
    assert_eq!(&bytes[4..260], &signature);
    let mut expected = Xxh3::new();
    expected.update(&VERSION);
    for (name, data) in [
        (0x10u64, &b"shared"[..]),
        (0x20, b"other"),
        (0x30, b"shared"),
    ] {
        expected.update(&name.to_le_bytes());
        expected.update(&content_checksum(data).to_le_bytes());
    }
    assert_eq!(
        u64::from_le_bytes(bytes[260..268].try_into().expect("8 bytes")),
        expected.digest(),
        "header checksum is XXH3(version, (name, checksum)...)"
    );

    let archive = WadArchive::parse(&bytes).expect("parse");
    let names: Vec<u64> = archive.entries().iter().map(|e| e.path_hash).collect();
    assert_eq!(names, [0x10, 0x20, 0x30]);
    let first = archive.find_by_hash(0x10).expect("0x10");
    let third = archive.find_by_hash(0x30).expect("0x30");
    assert_eq!(first.offset, third.offset, "same checksum, one copy");
    assert_eq!(
        bytes.len(),
        WAD_HEADER_SIZE + 3 * WAD_ENTRY_SIZE + 6 + 5,
        "the shared payload is stored once"
    );
    assert_eq!(archive.read_entry(first).expect("read"), b"shared");
}

#[test]
fn test_memory_payloads_are_deduplicated_by_their_actual_bytes_not_a_carried_checksum() {
    let mut writer = WadWriter::default();

    let mut a = memory(0, b"aaaaaa", 6);
    let mut b = memory(0, b"bbbbbb", 6);
    a.checksum = 42;
    b.checksum = 42;

    let mut a_dup = memory(0, b"aaaaaa", 6);
    a_dup.checksum = 999;
    writer.insert(0x10, a);
    writer.insert(0x20, b);
    writer.insert(0x30, a_dup);
    let bytes = writer.to_bytes().expect("write");

    let archive = WadArchive::parse(&bytes).expect("parse");
    let ea = archive.find_by_hash(0x10).expect("0x10");
    let eb = archive.find_by_hash(0x20).expect("0x20");
    let edup = archive.find_by_hash(0x30).expect("0x30");
    assert_ne!(
        ea.offset, eb.offset,
        "different bytes never share a location"
    );
    assert_eq!(
        ea.offset, edup.offset,
        "identical bytes share one copy, whatever the carried checksum says"
    );
    assert_eq!(archive.read_entry(ea).expect("read"), b"aaaaaa");
    assert_eq!(archive.read_entry(eb).expect("read"), b"bbbbbb");
}

#[test]
fn test_file_payloads_are_deduplicated_by_location_not_the_games_checksum() {
    let dir = temp_dir("dedup_file");

    let src = dir.join("src.bin");
    std::fs::write(&src, b"AAAABBBB").expect("write src");
    let mut writer = WadWriter::default();
    let source = writer.add_source(&src);

    writer.insert(
        0x10,
        WriterEntry {
            kind: 0,
            subchunk_count: 0,
            first_subchunk: 0,
            uncompressed_size: 4,
            checksum: 0,
            payload: Payload::File {
                source,
                offset: 0,
                len: 4,
            },
        },
    );
    writer.insert(
        0x20,
        WriterEntry {
            kind: 0,
            subchunk_count: 0,
            first_subchunk: 0,
            uncompressed_size: 4,
            checksum: 0,
            payload: Payload::File {
                source,
                offset: 4,
                len: 4,
            },
        },
    );
    let out = dir.join("out.wad.client");
    writer
        .write_to_file(&out, &|| false)
        .expect("write to file");
    let archive = WadFile::open(&out).expect("open");
    assert_eq!(
        archive.read(0x10).expect("read").as_deref(),
        Some(&b"AAAA"[..])
    );
    assert_eq!(
        archive.read(0x20).expect("read").as_deref(),
        Some(&b"BBBB"[..]),
        "a colliding game checksum must not serve the first entry's bytes"
    );
}

#[test]
fn test_entries_are_copied_from_a_source_file_and_an_identical_wad_is_not_rewritten() {
    let dir = temp_dir("copy");

    let mut game = WadWriter::default();
    let zstd = zstd::bulk::compress(b"texture bytes", 3).expect("zstd");
    game.insert(0xA, memory(3, &zstd, 13));
    game.insert(0xB, memory(0, b"plain", 5));
    let game_path = dir.join("Game.wad.client");
    std::fs::write(&game_path, game.to_bytes().expect("game")).expect("write game");

    let source = WadFile::open(&game_path).expect("open");
    let mut overlay = WadWriter::new(*source.signature());
    let index = overlay.add_source(&game_path);
    for entry in source.toc() {
        overlay.insert(entry.path_hash, WriterEntry::from_wad(index, entry));
    }
    overlay.insert(0xB, optimal_raw(b"modded".to_vec()).expect("optimal"));
    let out = dir.join("out").join("Game.wad.client");

    let first = overlay.write_to_file(&out, &|| false).expect("write");
    assert!(matches!(first, WriteOutcome::Written { .. }));
    let written = WadFile::open(&out).expect("reopen");
    assert_eq!(
        written.read(0xA).expect("a").as_deref(),
        Some(&b"texture bytes"[..])
    );
    assert_eq!(
        written.read(0xB).expect("b").as_deref(),
        Some(&b"modded"[..])
    );
    assert!(!partial_path(&out).exists(), "no partial file left behind");

    let second = overlay.write_to_file(&out, &|| false).expect("again");
    assert_eq!(
        second,
        WriteOutcome::Unchanged {
            bytes: first.bytes()
        }
    );

    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: fixture cleanup
}

#[test]
fn test_a_cancelled_write_leaves_the_destination_untouched() {
    let dir = temp_dir("cancel");
    let out = dir.join("X.wad.client");
    std::fs::write(&out, b"previous").expect("previous");
    let mut writer = WadWriter::default();
    writer.insert(1, memory(0, b"data", 4));
    let result = writer.write_to_file(&out, &|| true);
    assert!(matches!(result, Err(WadError::Cancelled)));
    assert_eq!(std::fs::read(&out).expect("read"), b"previous");
    assert!(!partial_path(&out).exists());
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: fixture cleanup
}

#[test]
fn test_mod_entries_take_the_form_mkoverlay_writes() {
    let texture = optimal_raw(b"DDS texture data".to_vec()).expect("texture");
    assert_eq!(texture.kind, CompressionType::Zstd as u8);
    let bank = optimal_raw(b"BKHD wwise bank".to_vec()).expect("bank");
    assert_eq!(bank.kind, CompressionType::Raw as u8);
    let wpk = optimal_raw(b"r3d2\x01\x00\x00\x00".to_vec()).expect("wpk");
    assert_eq!(wpk.kind, CompressionType::Raw as u8);
    let model = optimal_raw(b"r3d2Mesh model".to_vec()).expect("model");
    assert_eq!(
        model.kind,
        CompressionType::Zstd as u8,
        "r3d2Mesh is a model, not audio"
    );

    let entry = |compression, size| WadEntry {
        path_hash: 7,
        offset: 0,
        compressed_size: 0,
        uncompressed_size: size,
        compression,
        checksum: 0,
        subchunk_count: 2,
        first_subchunk: 9,
    };

    let chunked = optimal_stored(&entry(CompressionType::ZstdChunked, 5), vec![1, 2], || {
        Ok(b"hello".to_vec())
    })
    .expect("chunked");
    assert_eq!(chunked.kind, CompressionType::Zstd as u8);
    assert_eq!((chunked.subchunk_count, chunked.first_subchunk), (0, 0));

    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(b"gzipped").expect("gz");
    let gz = gz.finish().expect("gz finish");
    for kind in [CompressionType::Gzip, CompressionType::Redirection] {
        let converted =
            optimal_stored(&entry(kind, 7), gz.clone(), || unreachable!()).expect("gzip");
        assert_eq!(converted.kind, CompressionType::Zstd as u8);
    }

    let link = optimal_stored(
        &entry(CompressionType::Redirection, 12),
        b"DATA/x.bin\0\0".to_vec(),
        || unreachable!(),
    )
    .expect("link");
    assert_eq!(link.kind, CompressionType::Redirection as u8);

    let frame = zstd::bulk::compress(b"mesh data", 3).expect("zstd");
    let kept =
        optimal_stored(&entry(CompressionType::Zstd, 9), frame, || unreachable!()).expect("z");
    assert_eq!(kept.kind, CompressionType::Zstd as u8);
    let bank_frame = zstd::bulk::compress(b"BKHD bank", 3).expect("zstd");
    let bank = optimal_stored(&entry(CompressionType::Zstd, 9), bank_frame, || {
        Ok(b"BKHD bank".to_vec())
    })
    .expect("bank");
    assert_eq!(bank.kind, CompressionType::Raw as u8);
}
