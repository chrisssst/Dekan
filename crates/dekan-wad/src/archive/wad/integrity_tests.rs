use super::*;
use crate::hash::content_checksum;
use xxhash_rust::xxh3::xxh3_64;

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
fn test_rejects_unsupported_minor_version() {
    let mut data = vec![0u8; WAD_HEADER_SIZE];
    data[0] = b'R';
    data[1] = b'W';
    data[2] = 3;

    data[3] = 255;
    let err = WadArchive::parse(&data).unwrap_err();
    match err {
        WadError::UnsupportedVersion(maj, min) => {
            assert_eq!(maj, 3);
            assert_eq!(min, 255);
        }
        other => panic!("expected UnsupportedVersion(3, 255), got {other:?}"),
    }

    let temp_path =
        std::env::temp_dir().join(format!("dekan_wad_minor255_{}.wad", std::process::id()));
    std::fs::write(&temp_path, &data).expect("write temp wad");
    let file_err = WadFile::open(&temp_path).unwrap_err();
    match file_err {
        WadError::UnsupportedVersion(maj, min) => {
            assert_eq!(maj, 3);
            assert_eq!(min, 255);
        }
        other => panic!("expected UnsupportedVersion(3, 255) from WadFile::open, got {other:?}"),
    }
    let _ = std::fs::remove_file(&temp_path); // ignore-ok: cleanup temporary fixture file

    data[3] = 5;
    let err5 = WadArchive::parse(&data).unwrap_err();
    assert!(matches!(err5, WadError::UnsupportedVersion(3, 5)));

    for minor in 0..=4 {
        data[3] = minor;
        let archive = WadArchive::parse(&data).expect("supported minor version must parse");
        assert_eq!(archive.header().minor, minor);
    }
}

#[test]
fn test_rejects_decompression_size_mismatch() {
    let raw_payload = b"Exact ten!";
    assert_eq!(raw_payload.len(), 10);

    let wad_bytes = build_synthetic_wad(&[(0x9999, CompressionType::Raw, raw_payload, 100)]);
    let archive = WadArchive::parse(&wad_bytes).expect("parse TOC succeeds");
    let entry = archive.find_by_hash(0x9999).expect("find entry");

    let err = archive.read_entry(entry).unwrap_err();
    match err {
        WadError::SizeMismatch {
            path_hash,
            declared,
            actual,
        } => {
            assert_eq!(path_hash, 0x9999);
            assert_eq!(declared, 100);
            assert_eq!(actual, 10);
        }
        other => panic!("expected SizeMismatch, got {other:?}"),
    }
}

#[test]
fn test_wad_checksum_specification_and_mod_tolerance() {
    let payload = b"Skin mesh and skeleton binary data";
    let computed_xxh3 = xxh3_64(payload);
    let helper_hash = content_checksum(payload);
    assert_eq!(computed_xxh3, helper_hash);

    let mut wad_bytes =
        build_synthetic_wad(&[(0x1234, CompressionType::Raw, payload, payload.len())]);
    let entry_offset = WAD_HEADER_SIZE;
    wad_bytes[entry_offset + 24..entry_offset + 32].copy_from_slice(&0u64.to_le_bytes());

    let archive = WadArchive::parse(&wad_bytes).expect("WAD with zero checksum in TOC must parse");
    let entry = archive.find_by_hash(0x1234).expect("entry exists");
    assert_eq!(entry.checksum, 0);
    let data = archive
        .read_entry(entry)
        .expect("entry decompresses cleanly");
    assert_eq!(data, payload);
}
