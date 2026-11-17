use std::io::{Cursor, Write};
use std::sync::Arc;
use std::time::Instant;

use dekan_wad::WadFile;
use dekan_wad::error::WadError;
use dekan_wad::fantome::{
    ModArchiveShape, mod_archive_shape, wad_mount_alias, wad_names_in_archive,
};
use zip::write::SimpleFileOptions;

fn create_test_wad(entry_count: u32) -> Vec<u8> {
    let mut data = Vec::new();
    data.extend_from_slice(b"RW");
    data.push(3);
    data.push(4);
    data.extend_from_slice(&[0u8; 256]);
    data.extend_from_slice(&[0u8; 8]);
    data.extend_from_slice(&entry_count.to_le_bytes());

    let header_size = 272u32;
    let toc_size = entry_count * 32;
    let data_offset = header_size + toc_size;

    for i in 0..entry_count {
        data.extend_from_slice(&(i as u64 + 1).to_le_bytes());
        let offset = data_offset + (i * 10);
        data.extend_from_slice(&offset.to_le_bytes());
        data.extend_from_slice(&10u32.to_le_bytes());
        data.extend_from_slice(&10u32.to_le_bytes());
        data.push(0);
        data.push(0);
        data.extend_from_slice(&[0u8; 2]);
        data.extend_from_slice(&[0u8; 8]);
    }

    for _ in 0..entry_count {
        data.extend_from_slice(b"0123456789");
    }

    data
}

fn build_synthetic_fantome_package(
    mod_name: &str,
    champion: &str,
    skin_id: u32,
    chroma_id: Option<u32>,
    wad_payload: &[u8],
) -> Vec<u8> {
    let mut buffer = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(Cursor::new(&mut buffer));
        let options = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .unix_permissions(0o755);

        let info_json = serde_json::json!({
            "Name": mod_name,
            "Author": "CommunityModder",
            "Version": "1.0.0",
            "Description": "High-fidelity custom skin community package",
            "Champion": champion,
            "SkinID": skin_id,
            "ChromaID": chroma_id
        });
        let info_bytes = serde_json::to_vec_pretty(&info_json).expect("meta");
        zip.start_file("META/info.json", options)
            .expect("start meta");
        zip.write_all(&info_bytes).expect("write meta");

        let wad_entry_name = if let Some(chroma) = chroma_id {
            format!("WAD/{champion}.skin_{skin_id}_chroma_{chroma}.wad.client")
        } else {
            format!("WAD/{champion}.wad.client")
        };
        zip.start_file(&wad_entry_name, options).expect("start wad");
        zip.write_all(wad_payload).expect("write wad payload");

        zip.start_file(
            format!("RAW/assets/characters/{champion}/skins/skin{skin_id}/texture.dds"),
            options,
        )
        .expect("start raw");
        zip.write_all(b"DDS_TEXTURE_PAYLOAD_DATA")
            .expect("write raw");

        zip.finish().expect("finish zip");
    }
    buffer
}

#[test]
fn test_custom_skin_package_import_and_shape_analysis() {
    let wad_bytes = create_test_wad(1);
    let fantome =
        build_synthetic_fantome_package("Cosmic Blade Master", "MasterYi", 5, None, &wad_bytes);

    let shape = mod_archive_shape(Cursor::new(&fantome)).expect("shape parse");
    assert_eq!(
        shape,
        ModArchiveShape {
            manifest: true,
            content: true,
        }
    );

    let wads = wad_names_in_archive(Cursor::new(&fantome)).expect("wads in archive");
    assert!(wads.contains("MasterYi"));
    assert_eq!(wad_mount_alias("MasterYi.wad.client"), "MasterYi");
}

#[test]
fn test_custom_skin_chroma_and_multi_skin_packaging() {
    let wad_bytes = create_test_wad(2);
    let fantome_chroma = build_synthetic_fantome_package(
        "Project Vayne Ruby Chroma",
        "Vayne",
        11,
        Some(3),
        &wad_bytes,
    );

    let shape = mod_archive_shape(Cursor::new(&fantome_chroma)).expect("shape");
    assert!(shape.manifest);
    assert!(shape.content);

    let wads = wad_names_in_archive(Cursor::new(&fantome_chroma)).expect("wads");
    assert!(wads.contains("Vayne.skin_11_chroma_3"));
}

#[test]
fn test_custom_skin_wad_v34_parsing_and_integrity_check() {
    let temp_dir = std::env::temp_dir().join(format!("dekan_skin_test_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&temp_dir); // ignore-ok: fixture dir
    let wad_path = temp_dir.join("CustomSkin.wad.client");

    let valid_wad = create_test_wad(3);
    std::fs::write(&wad_path, valid_wad).expect("write valid wad");

    let opened = WadFile::open(&wad_path).expect("open custom skin wad");
    assert_eq!(opened.len(), 3);
    assert!(!opened.is_empty());

    let mut corrupted = create_test_wad(3);
    corrupted[0] = b'X';
    corrupted[1] = b'Y';
    std::fs::write(&wad_path, corrupted).expect("write corrupted wad");

    let err = WadFile::open(&wad_path);
    assert!(matches!(err, Err(WadError::InvalidMagic(_))));

    let _ = std::fs::remove_dir_all(&temp_dir); // ignore-ok: cleanup
}

#[test]
fn test_custom_skin_concurrent_resolution_and_memory_bench() {
    let temp_dir = std::env::temp_dir().join(format!("dekan_bench_skin_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&temp_dir); // ignore-ok: fixture dir
    let wad_path = temp_dir.join("BenchSkin.wad.client");

    let valid_wad = create_test_wad(10);
    std::fs::write(&wad_path, valid_wad).expect("write bench wad");

    let opened = Arc::new(WadFile::open(&wad_path).expect("open"));
    let mut handles = Vec::new();
    let start = Instant::now();

    for _ in 0..8 {
        let archive = Arc::clone(&opened);
        handles.push(std::thread::spawn(move || {
            for _ in 0..100 {
                let count = archive.len();
                assert_eq!(count, 10);
            }
        }));
    }

    for handle in handles {
        handle.join().expect("thread join");
    }

    let elapsed = start.elapsed();
    assert!(elapsed.as_millis() < 2000);

    let _ = std::fs::remove_dir_all(&temp_dir); // ignore-ok: cleanup
}

#[test]
fn test_malformed_custom_skin_packages_fail_closed() {
    let empty_zip: Vec<u8> = {
        let mut buf = Vec::new();
        let zip = zip::ZipWriter::new(Cursor::new(&mut buf));
        zip.finish().expect("finish");
        buf
    };

    let shape = mod_archive_shape(Cursor::new(&empty_zip)).expect("shape");
    assert!(!shape.manifest);
    assert!(!shape.content);

    let invalid_bytes = b"NOT_A_VALID_ZIP_ARCHIVE_AT_ALL";
    let shape_err = mod_archive_shape(Cursor::new(invalid_bytes));
    assert!(matches!(shape_err, Err(WadError::InvalidFantome(_))));
}
