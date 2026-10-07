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

#[test]
fn test_a_rebased_wad_keeps_the_games_signature_and_checksum() {
    let dir = temp_dir("rebased");
    let mut game = WadWriter::new([7u8; WAD_SIGNATURE_SIZE]);
    game.insert(0xA, memory(0, b"base", 4));
    let game_path = dir.join("Game.wad.client");
    let mut bytes = game.to_bytes().expect("game");
    bytes[260..268].copy_from_slice(&0x1122_3344_5566_7788u64.to_le_bytes());
    std::fs::write(&game_path, &bytes).expect("write game");

    let source = WadFile::open(&game_path).expect("open");
    assert_eq!(source.checksum(), 0x1122_3344_5566_7788);
    let mut overlay = WadWriter::rebased_on(&source);
    overlay.insert(0xA, memory(0, b"mod!", 4));
    let header = overlay.header().expect("header");
    assert_eq!(&header[..268], &bytes[..268]);
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: fixture cleanup
}

#[test]
fn test_a_kept_checksum_never_hides_a_content_change() {
    let dir = temp_dir("kept_checksum");
    let mut game = WadWriter::new([7u8; WAD_SIGNATURE_SIZE]);
    game.insert(0xA, memory(0, b"base", 4));
    let game_path = dir.join("Game.wad.client");
    std::fs::write(&game_path, game.to_bytes().expect("game")).expect("write game");
    let source = WadFile::open(&game_path).expect("open");
    let out = dir.join("out").join("Game.wad.client");

    let mut first = WadWriter::rebased_on(&source);
    first.insert(0xA, memory(0, b"skin", 4));
    first.write_to_file(&out, &|| false).expect("first");
    let mut second = WadWriter::rebased_on(&source);
    second.insert(0xA, memory(0, b"odd!", 4));
    let outcome = second.write_to_file(&out, &|| false).expect("second");

    assert!(matches!(outcome, WriteOutcome::Written { .. }));
    assert_eq!(
        WadFile::open(&out)
            .expect("reopen")
            .read(0xA)
            .expect("a")
            .as_deref(),
        Some(&b"odd!"[..])
    );
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: fixture cleanup
}

fn game_wad(dir: &Path) -> PathBuf {
    let mut game = WadWriter::new([7u8; WAD_SIGNATURE_SIZE]);
    game.insert(0xA, memory(0, b"model", 5));
    game.insert(0xB, memory(0, b"shadow skin", 11));
    game.insert(0xC, memory(0, b"terrain", 7));
    let path = dir.join("Map11.wad.client");
    let mut bytes = game.to_bytes().expect("game");
    bytes[260..268].copy_from_slice(&0xABCD_EF01_2345_6789u64.to_le_bytes());
    std::fs::write(&path, bytes).expect("write game");
    path
}

fn rebased(game_path: &Path, replacements: &[(u64, &[u8])]) -> WadWriter {
    let source = WadFile::open(game_path).expect("open");
    let mut writer = WadWriter::rebased_on(&source);
    let index = writer.add_source(game_path);
    for entry in source.toc() {
        writer.insert(entry.path_hash, WriterEntry::from_wad(index, entry));
    }
    for (name, bytes) in replacements {
        writer.insert(*name, memory(0, bytes, bytes.len() as u64));
    }
    writer
}

#[test]
fn test_a_replacement_only_wad_is_the_game_copy_plus_its_new_entries() {
    let dir = temp_dir("over_copy");
    let game_path = game_wad(&dir);
    let game_bytes = std::fs::read(&game_path).expect("game bytes");
    let out = dir.join("out").join("Map11.wad.client");

    let outcome = rebased(&game_path, &[(0xB, b"new shadow skin")])
        .write_over_game_copy(&out, "4", &|| false)
        .expect("write")
        .expect("eligible");
    assert!(matches!(outcome, WriteOutcome::Written { .. }));

    let written = std::fs::read(&out).expect("overlay");
    assert_eq!(
        &written[..272],
        &game_bytes[..272],
        "the game's header, untouched"
    );
    let data_start = 272 + 3 * WAD_ENTRY_SIZE;
    assert_eq!(
        &written[data_start..game_bytes.len()],
        &game_bytes[data_start..],
        "every game byte stays where the game has it"
    );
    let wad = WadFile::open(&out).expect("reopen");
    assert_eq!(
        wad.read(0xB).expect("b").as_deref(),
        Some(&b"new shadow skin"[..])
    );
    assert_eq!(wad.read(0xA).expect("a").as_deref(), Some(&b"model"[..]));
    assert_eq!(wad.read(0xC).expect("c").as_deref(), Some(&b"terrain"[..]));
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: fixture cleanup
}

#[test]
fn test_the_game_copy_is_reused_until_the_game_file_changes() {
    let dir = temp_dir("over_copy_reuse");
    let game_path = game_wad(&dir);
    let out = dir.join("out").join("Map11.wad.client");
    rebased(&game_path, &[(0xB, b"first skin")])
        .write_over_game_copy(&out, "4", &|| false)
        .expect("first")
        .expect("eligible");

    let mut marker = std::fs::OpenOptions::new()
        .write(true)
        .open(&out)
        .expect("open copy");
    marker
        .seek(SeekFrom::Start(272 + 3 * WAD_ENTRY_SIZE as u64 + 1))
        .expect("seek");
    marker.write_all(b"Z").expect("mark the copy");
    drop(marker);

    rebased(&game_path, &[(0xB, b"second")])
        .write_over_game_copy(&out, "4", &|| false)
        .expect("second")
        .expect("eligible");
    let reused = std::fs::read(&out).expect("overlay");
    assert_eq!(
        reused[272 + 3 * WAD_ENTRY_SIZE + 1],
        b'Z',
        "the copy was not made again"
    );
    assert_eq!(
        WadFile::open(&out)
            .expect("reopen")
            .read(0xB)
            .expect("b")
            .as_deref(),
        Some(&b"second"[..]),
        "the old tail is cut and the new entry lands"
    );

    let mut game = std::fs::read(&game_path).expect("game");
    game.push(0);
    std::fs::write(&game_path, game).expect("patched game");
    rebased(&game_path, &[(0xB, b"third")])
        .write_over_game_copy(&out, "4", &|| false)
        .expect("third")
        .expect("eligible");
    assert_ne!(
        std::fs::read(&out).expect("overlay")[272 + 3 * WAD_ENTRY_SIZE + 1],
        b'Z',
        "a changed game file is copied again"
    );
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: fixture cleanup
}

#[test]
fn test_a_mod_that_adds_entries_is_not_written_over_a_copy() {
    let dir = temp_dir("over_copy_added");
    let game_path = game_wad(&dir);
    let out = dir.join("out").join("Map11.wad.client");
    let outcome = rebased(&game_path, &[(0xD, b"brand new")])
        .write_over_game_copy(&out, "4", &|| false)
        .expect("checked");
    assert_eq!(outcome, None);
    assert!(!out.exists());
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: fixture cleanup
}
