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
