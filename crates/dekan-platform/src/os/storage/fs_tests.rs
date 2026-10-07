use super::*;
use std::io::Cursor;
use zip::write::{SimpleFileOptions, ZipWriter};

#[test]
fn test_atomic_write_creates_and_overwrites() {
    let temp_dir = std::env::temp_dir().join("dekan_test_atomic_write");
    let target = temp_dir.join("config.json");

    atomic_write(&target, b"{\"version\": 1}", true).expect("first write");
    assert_eq!(std::fs::read(&target).unwrap(), b"{\"version\": 1}");

    atomic_write(&target, b"{\"version\": 2}", true).expect("second write");
    assert_eq!(std::fs::read(&target).unwrap(), b"{\"version\": 2}");

    std::fs::remove_dir_all(&temp_dir).ok();
}

#[test]
fn test_disk_free_space() {
    let temp = std::env::temp_dir();
    let free = get_disk_free_space(&temp).expect("query free space");
    assert!(free > 0, "disk free space should be non-zero");
}

#[test]
fn test_validate_archive_path() {
    assert!(validate_archive_path(Path::new("normal/file.txt")).is_ok());
    assert!(validate_archive_path(Path::new("a/b/c.json")).is_ok());

    assert!(validate_archive_path(Path::new("../evil.txt")).is_err());
    assert!(validate_archive_path(Path::new("a/../../evil.exe")).is_err());
    assert!(validate_archive_path(Path::new("/absolute/path")).is_err());
    assert!(validate_archive_path(Path::new("C:\\Windows\\System32")).is_err());
}

#[test]
fn test_safe_extract_zip_valid_and_zip_bomb() {
    let mut zip_bytes = Vec::new();
    {
        let mut writer = ZipWriter::new(Cursor::new(&mut zip_bytes));
        let options = SimpleFileOptions::default();

        writer.start_file("data/test.txt", options).unwrap();
        writer.write_all(b"safe decompressed content").unwrap();
        writer.finish().unwrap();
    }

    let temp_dest = std::env::temp_dir().join("dekan_test_safe_extract");

    let count = safe_extract_zip(
        Cursor::new(&zip_bytes),
        &temp_dest,
        &ExtractLimits::default(),
    )
    .unwrap();
    assert_eq!(count, 1);
    assert_eq!(
        std::fs::read(temp_dest.join("data/test.txt")).unwrap(),
        b"safe decompressed content"
    );

    let tiny_limits = ExtractLimits {
        max_total_bytes: 10,
        max_single_file_bytes: 10,
        max_entries: 10,
        max_path_len: MAX_PATH_CHARS,
    };
    let err = safe_extract_zip(Cursor::new(&zip_bytes), &temp_dest, &tiny_limits).unwrap_err();
    assert!(matches!(err, PlatformError::Security(_)));

    let tiny_path_limits = ExtractLimits {
        max_path_len: 5,
        ..ExtractLimits::default()
    };
    let err = safe_extract_zip(Cursor::new(&zip_bytes), &temp_dest, &tiny_path_limits).unwrap_err();
    assert!(matches!(err, PlatformError::Security(_)));

    let _ = std::fs::remove_dir_all(&temp_dest); // ignore-ok: cleanup test directory
}

#[test]
fn test_mirror_tree_links_files_and_leaves_the_source_intact() {
    let root = std::env::temp_dir().join(format!("dekan_mirror_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root); // ignore-ok: cleanup any preexisting test directory
    let src = root.join("src");
    std::fs::create_dir_all(src.join("META")).unwrap();
    std::fs::create_dir_all(src.join("WAD").join("Nested")).unwrap();
    std::fs::write(src.join("META").join("info.json"), b"{}").unwrap();
    std::fs::write(src.join("WAD").join("Nested").join("a.bin"), b"payload").unwrap();

    let dst = root.join("dst");
    let stats = mirror_tree(&src, &dst).unwrap();
    assert_eq!(stats.linked + stats.copied, 2);
    assert_eq!(
        std::fs::read(dst.join("WAD").join("Nested").join("a.bin")).unwrap(),
        b"payload"
    );

    assert!(mirror_tree(&src, &dst).is_err());

    std::fs::remove_dir_all(&dst).unwrap();
    assert!(src.join("WAD").join("Nested").join("a.bin").is_file());

    let _ = std::fs::remove_dir_all(&root); // ignore-ok: fixture cleanup
}

#[test]
fn test_an_entry_with_a_wrong_crc_is_still_extracted_whole() {
    let content = b"mod files written by a tool that gets the crc wrong";
    let mut zip_bytes = Vec::new();
    {
        let mut writer = ZipWriter::new(Cursor::new(&mut zip_bytes));
        let stored =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        writer.start_file("WAD/Zed.wad.client", stored).unwrap();
        writer.write_all(content).unwrap();
        writer.finish().unwrap();
    }
    let crc = zip::ZipArchive::new(Cursor::new(&zip_bytes))
        .unwrap()
        .by_index(0)
        .unwrap()
        .crc32()
        .to_le_bytes();
    let wrong = (u32::from_le_bytes(crc) ^ 0xFFFF_FFFF).to_le_bytes();
    let mut patched = 0;
    for at in 0..zip_bytes.len() - 3 {
        if zip_bytes[at..at + 4] == crc {
            zip_bytes[at..at + 4].copy_from_slice(&wrong);
            patched += 1;
        }
    }
    assert_eq!(patched, 2, "local header and central directory");

    let dest = std::env::temp_dir().join(format!("dekan_test_bad_crc_{}", std::process::id()));
    let count = safe_extract_zip(Cursor::new(&zip_bytes), &dest, &ExtractLimits::default())
        .expect("a wrong crc does not refuse the archive");
    assert_eq!(count, 1);
    assert_eq!(
        std::fs::read(dest.join("WAD/Zed.wad.client")).unwrap(),
        content
    );
    let _ = std::fs::remove_dir_all(&dest); // ignore-ok: cleanup test directory
}

#[test]
fn names_windows_treats_specially_are_refused_and_accepted_paths_stay_inside() {
    for hostile in [
        "WAD/Zed.wad.client:stream",
        "WAD/CON",
        "WAD/nul.txt",
        "WAD/com1.wad.client",
        "WAD/trailing.",
        "WAD/trailing ",
        "WAD/a<b",
        "WAD/a|b",
        "WAD/a?b",
        "WAD/a*b",
        "WAD/a\"b",
        "WAD/tab\tname",
        "C:evil",
        r"\\server\share\x",
    ] {
        assert!(
            validate_archive_path(Path::new(hostile)).is_err(),
            "{hostile:?}"
        );
    }
    for fine in [
        "WAD/Zed.wad.client",
        "META/info.json",
        "RAW/assets/a b/c.dds",
        "WAD/console.wad.client",
    ] {
        assert!(validate_archive_path(Path::new(fine)).is_ok(), "{fine:?}");
    }

    let mut state = 0x5EED_0006u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let pieces = [
        "..", ".", "a", "B", ":", "\\", "/", "CON", "nul", " ", "x.", "<", "|", "\u{1}", "é", "c:",
    ];
    let dest = Path::new(r"C:\Dekan\mods\staged");
    for _ in 0..20_000 {
        let candidate: String = (0..next() % 8 + 1)
            .map(|_| pieces[(next() % pieces.len() as u64) as usize])
            .collect();
        if let Ok(clean) = validate_archive_path(Path::new(&candidate)) {
            let target = dest.join(&clean);
            assert!(
                target.starts_with(dest),
                "{candidate:?} escaped to {target:?}"
            );
            assert!(
                clean
                    .components()
                    .all(|c| matches!(c, Component::Normal(_))),
                "{candidate:?}"
            );
            let text = clean.to_string_lossy();
            assert!(
                !text.contains(':') && !text.contains('|') && !text.contains('<'),
                "{candidate:?}"
            );
        }
    }
}
