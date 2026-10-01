use super::*;
use crate::hash::content_checksum;

#[test]
fn test_wad_file_reads_single_entries_from_disk() {
    let wad = build_synthetic_wad(&[
        (0xAAAA, CompressionType::Raw, b"first", 5),
        (0xBBBB, CompressionType::Raw, b"second", 6),
    ]);
    let path = std::env::temp_dir().join(format!("dekan_wadfile_{}.wad", std::process::id()));
    std::fs::write(&path, &wad).expect("write wad");

    let file = WadFile::open(&path).expect("open");
    assert_eq!(file.len(), 2);
    assert!(file.contains(0xBBBB));
    assert_eq!(
        file.read(0xBBBB).expect("read").as_deref(),
        Some(&b"second"[..])
    );
    assert_eq!(file.read(0xCCCC).expect("absent is not an error"), None);

    std::fs::write(&path, &wad[..wad.len() - 3]).expect("truncate");
    assert!(matches!(
        WadFile::open(&path),
        Err(WadError::OffsetOutOfRange { .. })
    ));

    std::fs::remove_file(&path).ok();
    assert!(matches!(WadFile::open(&path), Err(WadError::FileIo { .. })));
}

fn build_synthetic_wad(entries_data: &[(u64, CompressionType, &[u8], usize)]) -> Vec<u8> {
    let entry_count = entries_data.len();
    let toc_size = entry_count * WAD_ENTRY_SIZE;
    let mut buffer = vec![0u8; WAD_HEADER_SIZE + toc_size];

    buffer[0] = b'R';
    buffer[1] = b'W';
    buffer[2] = 3;
    buffer[3] = 4;
    buffer[268..272].copy_from_slice(&(entry_count as u32).to_le_bytes());

    let mut current_payload_offset = buffer.len();
    for (idx, &(hash, comp_type, payload, uncomp_len)) in entries_data.iter().enumerate() {
        let entry_offset = WAD_HEADER_SIZE + (idx * WAD_ENTRY_SIZE);
        let comp_len = payload.len();

        let mut entry_bytes = [0u8; 32];
        entry_bytes[0..8].copy_from_slice(&hash.to_le_bytes());
        entry_bytes[8..12].copy_from_slice(&(current_payload_offset as u32).to_le_bytes());
        entry_bytes[12..16].copy_from_slice(&(comp_len as u32).to_le_bytes());
        entry_bytes[16..20].copy_from_slice(&(uncomp_len as u32).to_le_bytes());
        entry_bytes[20] = comp_type as u8;
        entry_bytes[24..32].copy_from_slice(&content_checksum(payload).to_le_bytes());

        buffer[entry_offset..entry_offset + 32].copy_from_slice(&entry_bytes);
        buffer.extend_from_slice(payload);
        current_payload_offset += comp_len;
    }

    buffer
}

#[test]
fn test_parse_empty_wad() {
    let mut data = vec![0u8; WAD_HEADER_SIZE];
    data[0] = b'R';
    data[1] = b'W';
    data[2] = 3;
    data[3] = 4;

    let archive = WadArchive::parse(&data).expect("should parse empty WAD");
    assert_eq!(archive.header().entry_count, 0);
    assert_eq!(archive.entries().len(), 0);
}

#[test]
fn test_rejects_invalid_magic() {
    let mut data = vec![0u8; WAD_HEADER_SIZE];
    data[0] = b'N';
    data[1] = b'O';
    data[2] = 3;

    let err = WadArchive::parse(&data).unwrap_err();
    match err {
        WadError::InvalidMagic(m) => assert_eq!(&m, b"NO"),
        other => panic!("expected InvalidMagic, got {other:?}"),
    }
}

#[test]
fn test_rejects_truncated_payload_offset() {
    let raw_data = b"Hello League of Legends";
    let mut wad_bytes =
        build_synthetic_wad(&[(0x1234_5678, CompressionType::Raw, raw_data, raw_data.len())]);

    wad_bytes.truncate(wad_bytes.len() - 10);

    let err = WadArchive::parse(&wad_bytes).unwrap_err();
    match err {
        WadError::OffsetOutOfRange { .. } => {}
        other => panic!("expected OffsetOutOfRange, got {other:?}"),
    }
}

#[test]
fn test_raw_and_zstd_decompression() {
    let original_data = b"Dekan skin changer in Rust: high performance and memory safety.";

    let zstd_data = zstd::encode_all(&original_data[..], 3).expect("zstd compression");

    let wad_bytes = build_synthetic_wad(&[
        (
            0xAAAA_BBBB,
            CompressionType::Raw,
            original_data,
            original_data.len(),
        ),
        (
            0xCCCC_DDDD,
            CompressionType::Zstd,
            &zstd_data,
            original_data.len(),
        ),
    ]);

    let archive = WadArchive::parse(&wad_bytes).expect("parse synthetic WAD");
    assert_eq!(archive.entries().len(), 2);

    let e1 = archive.find_by_hash(0xAAAA_BBBB).expect("find entry 1");
    let d1 = archive.read_entry(e1).expect("read raw entry");
    assert_eq!(d1, original_data);

    let e2 = archive.find_by_hash(0xCCCC_DDDD).expect("find entry 2");
    let d2 = archive.read_entry(e2).expect("read zstd entry");
    assert_eq!(d2, original_data);
}

#[test]
fn test_zstd_chunked_decodes_concatenated_frames() {
    let chunks: [&[u8]; 3] = [
        b"first subchunk of a skin bin",
        &[0x5Au8; 4096],
        b"last subchunk, shorter",
    ];
    let expected: Vec<u8> = chunks.concat();
    let payload: Vec<u8> = chunks
        .iter()
        .flat_map(|c| zstd::encode_all(*c, 3).expect("zstd compression"))
        .collect();

    let mut wad_bytes = build_synthetic_wad(&[(
        0xC4C4_C4C4,
        CompressionType::ZstdChunked,
        &payload,
        expected.len(),
    )]);
    wad_bytes[WAD_HEADER_SIZE + 20] = 0x34;

    let archive = WadArchive::parse(&wad_bytes).expect("parse synthetic WAD");
    let entry = archive.find_by_hash(0xC4C4_C4C4).expect("entry");
    assert_eq!(entry.compression, CompressionType::ZstdChunked);
    assert_eq!(archive.read_entry(entry).expect("decode type 4"), expected);

    let path = std::env::temp_dir().join(format!("dekan_wad_type4_{}.wad", std::process::id()));
    std::fs::write(&path, &wad_bytes).expect("write wad");
    let file = WadFile::open(&path).expect("open");
    assert_eq!(
        file.read(0xC4C4_C4C4).expect("read").as_deref(),
        Some(expected.as_slice())
    );
    std::fs::remove_file(&path).ok();
}

#[test]
fn test_zstd_chunked_truncated_frame_is_an_error() {
    let chunk = [0x33u8; 2048];
    let mut payload = zstd::encode_all(&chunk[..], 3).expect("zstd compression");
    payload.extend(zstd::encode_all(&chunk[..], 3).expect("zstd compression"));
    payload.truncate(payload.len() - 4);

    let wad_bytes = build_synthetic_wad(&[(
        0xDEAD,
        CompressionType::ZstdChunked,
        &payload,
        chunk.len() * 2,
    )]);
    let archive = WadArchive::parse(&wad_bytes).expect("parse synthetic WAD");
    let entry = archive.find_by_hash(0xDEAD).expect("entry");
    assert!(matches!(
        archive.read_entry(entry),
        Err(WadError::Decompression(_))
    ));
}

#[test]
fn test_subchunk_toc_name_follows_the_game_path() {
    let name = |p: &str| subchunk_toc_name(std::path::Path::new(p));
    assert_eq!(
        name(r"D:\Riot Games\League of Legends\Game\DATA\FINAL\Champions\Annie.wad.client")
            .as_deref(),
        Some("data/final/champions/annie.wad.subchunktoc")
    );
    assert_eq!(
        name("C:/x/DATA/FINAL/Maps/Shipping/Map22.wad.client").as_deref(),
        Some("data/final/maps/shipping/map22.wad.subchunktoc")
    );
    assert_eq!(name(r"C:\mods\Zed.wad.client"), None);
}

#[test]
fn test_type4_with_a_stored_subchunk_decodes_through_the_table() {
    let raw_part = b"TEX\0 header stored uncompressed".to_vec();
    let body = vec![0x77u8; 3000];
    let frame = zstd::encode_all(&body[..], 3).expect("zstd");
    let payload = [raw_part.clone(), frame.clone()].concat();
    let expected = [raw_part.clone(), body.clone()].concat();

    let mut toc = Vec::new();
    for (stored, target) in [
        (10u32, 20u32),
        (raw_part.len() as u32, raw_part.len() as u32),
        (frame.len() as u32, body.len() as u32),
    ] {
        toc.extend_from_slice(&stored.to_le_bytes());
        toc.extend_from_slice(&target.to_le_bytes());
        toc.extend_from_slice(&0u64.to_le_bytes());
    }
    let toc_name = "data/final/champions/test.wad.subchunktoc";
    let mut wad = build_synthetic_wad(&[
        (
            0x7E57,
            CompressionType::ZstdChunked,
            &payload,
            expected.len(),
        ),
        (
            crate::hash::wad_path_hash(toc_name),
            CompressionType::Raw,
            &toc,
            toc.len(),
        ),
    ]);

    wad[WAD_HEADER_SIZE + 20..WAD_HEADER_SIZE + 24].copy_from_slice(&[0x24, 0x00, 0x01, 0x00]);

    let mut archive = WadArchive::parse(&wad).expect("parse");
    let entry = archive.find_by_hash(0x7E57).cloned().expect("entry");
    assert_eq!((entry.subchunk_count, entry.first_subchunk), (2, 1));
    assert!(matches!(
        archive.read_entry(&entry),
        Err(WadError::Decompression(_))
    ));
    assert!(archive.load_subchunk_toc(toc_name));
    assert_eq!(archive.read_entry(&entry).expect("decode"), expected);

    let root = std::env::temp_dir().join(format!("dekan_subchunk_{}", std::process::id()));
    let dir = root.join("DATA").join("FINAL").join("Champions");
    std::fs::create_dir_all(&dir).expect("dir");
    let path = dir.join("Test.wad.client");
    std::fs::write(&path, &wad).expect("write");
    let file = WadFile::open(&path).expect("open");
    assert_eq!(
        file.read(0x7E57).expect("read").as_deref(),
        Some(expected.as_slice())
    );
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn test_a_table_that_does_not_cover_the_payload_is_refused() {
    let body = vec![0x11u8; 500];
    let frame = zstd::encode_all(&body[..], 3).expect("zstd");
    let payload = [b"RAW!".to_vec(), frame.clone()].concat();

    let mut toc = Vec::new();
    for (stored, target) in [(2u32, 2u32), (frame.len() as u32, body.len() as u32)] {
        toc.extend_from_slice(&stored.to_le_bytes());
        toc.extend_from_slice(&target.to_le_bytes());
        toc.extend_from_slice(&0u64.to_le_bytes());
    }
    let name = "data/final/x.wad.subchunktoc";
    let mut wad = build_synthetic_wad(&[
        (1, CompressionType::ZstdChunked, &payload, 4 + body.len()),
        (
            crate::hash::wad_path_hash(name),
            CompressionType::Raw,
            &toc,
            toc.len(),
        ),
    ]);
    wad[WAD_HEADER_SIZE + 20] = 0x24;
    let mut archive = WadArchive::parse(&wad).expect("parse");
    assert!(archive.load_subchunk_toc(name));
    let entry = archive.find_by_hash(1).cloned().expect("entry");
    assert!(matches!(
        archive.read_entry(&entry),
        Err(WadError::InvalidSubchunkToc(_) | WadError::Decompression(_))
    ));
}

#[test]
fn test_decoded_size_must_match_the_toc() {
    let chunk = [0x44u8; 1024];
    let frame = zstd::encode_all(&chunk[..], 3).expect("zstd compression");
    let raw = b"raw bytes";

    for (declared, compression, payload) in [
        (2048, CompressionType::ZstdChunked, frame.as_slice()),
        (512, CompressionType::ZstdChunked, frame.as_slice()),
        (512, CompressionType::Zstd, frame.as_slice()),
        (raw.len() + 1, CompressionType::Raw, raw.as_slice()),
    ] {
        let wad_bytes = build_synthetic_wad(&[(0xBEEF, compression, payload, declared)]);
        let archive = WadArchive::parse(&wad_bytes).expect("parse synthetic WAD");
        let entry = archive.find_by_hash(0xBEEF).expect("entry");
        match archive.read_entry(entry) {
            Err(WadError::SizeMismatch {
                declared: d,
                actual,
                ..
            }) => {
                assert_eq!(d, declared);

                assert!(
                    actual <= declared + 1,
                    "{compression:?} read {actual} bytes"
                );
            }
            other => {
                panic!("{compression:?} declared {declared}: expected SizeMismatch, got {other:?}")
            }
        }
    }
}
