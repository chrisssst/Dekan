use super::*;
use std::io::{Cursor, Write};
use std::path::Path;
use zip::write::{SimpleFileOptions, ZipWriter};

use crate::error::PlatformError;
use crate::os::storage::fs::MAX_PATH_CHARS;

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

    let temp_dest =
        std::env::temp_dir().join(format!("dekan_test_safe_extract_{}", std::process::id()));

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
    let limited_dest = std::env::temp_dir().join(format!(
        "dekan_test_safe_extract_limited_{}",
        std::process::id()
    ));
    let err = safe_extract_zip(Cursor::new(&zip_bytes), &limited_dest, &tiny_limits).unwrap_err();
    assert!(matches!(err, PlatformError::Security(_)));

    let tiny_path_limits = ExtractLimits {
        max_path_len: 5,
        ..ExtractLimits::default()
    };
    let path_limited_dest = std::env::temp_dir().join(format!(
        "dekan_test_safe_extract_path_limited_{}",
        std::process::id()
    ));
    let err = safe_extract_zip(
        Cursor::new(&zip_bytes),
        &path_limited_dest,
        &tiny_path_limits,
    )
    .unwrap_err();
    assert!(matches!(err, PlatformError::Security(_)));

    let _ = std::fs::remove_dir_all(&temp_dest); // ignore-ok: cleanup test directory
    let _ = std::fs::remove_dir_all(&limited_dest); // ignore-ok: cleanup test directory
    let _ = std::fs::remove_dir_all(&path_limited_dest); // ignore-ok: cleanup test directory
}

#[test]
fn unicode_archive_paths_round_trip_through_extraction() {
    let relative = "MODS/皮膚/Éowyn/skin_零.wad.client";
    let clean = validate_archive_path(Path::new(relative)).expect("unicode path is portable");
    assert_eq!(clean, Path::new(relative));

    let mut archive_bytes = Vec::new();
    {
        let mut writer = ZipWriter::new(Cursor::new(&mut archive_bytes));
        writer
            .start_file(relative, SimpleFileOptions::default())
            .expect("unicode zip entry");
        writer
            .write_all(b"unicode overlay fixture")
            .expect("entry bytes");
        writer.finish().expect("finish zip");
    }

    let root = std::env::temp_dir().join(format!(
        "dekan_test_unicode_archive_{}_\u{03bb}",
        std::process::id()
    ));
    let count = safe_extract_zip(
        Cursor::new(&archive_bytes),
        &root,
        &ExtractLimits::default(),
    )
    .expect("extract unicode entry");

    assert_eq!(count, 1);
    assert_eq!(
        std::fs::read(root.join(relative)).expect("read extracted unicode file"),
        b"unicode overlay fixture"
    );
    let _ = std::fs::remove_dir_all(&root); // ignore-ok: cleanup test directory
}

#[test]
fn test_partial_extraction_cleans_up_aborted_file() {
    let relative = "payload.bin";
    let mut archive_bytes = Vec::new();
    {
        let mut writer = ZipWriter::new(Cursor::new(&mut archive_bytes));
        writer
            .start_file(relative, SimpleFileOptions::default())
            .expect("zip entry");
        writer.write_all(&[0x42; 100]).expect("entry bytes");
        writer.finish().expect("finish zip");
    }

    let root =
        std::env::temp_dir().join(format!("dekan_test_partial_cleanup_{}", std::process::id()));

    let strict_limits = ExtractLimits {
        max_single_file_bytes: 50,
        ..ExtractLimits::default()
    };

    let result = safe_extract_zip(Cursor::new(&archive_bytes), &root, &strict_limits);
    assert!(
        result.is_err(),
        "extraction should fail due to exceeding limit"
    );

    let partial_path = root.join(relative);
    assert!(
        !partial_path.exists(),
        "partial file should be removed upon aborted extraction"
    );

    let _ = std::fs::remove_dir_all(&root); // ignore-ok: cleanup test fixture
}

#[test]
fn test_aborted_extraction_does_not_remove_unrelated_file() {
    let test_id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let outside_dir = std::env::temp_dir().join(format!("dekan_test_victim_{test_id}"));
    std::fs::create_dir_all(&outside_dir).expect("create outside dir");
    let victim_file = outside_dir.join("important_user_data.txt");
    std::fs::write(&victim_file, b"IMMUTABLE_USER_DATA").expect("write victim");

    let relative = "payload.bin";
    let mut archive_bytes = Vec::new();
    {
        let mut writer = ZipWriter::new(Cursor::new(&mut archive_bytes));
        writer
            .start_file(relative, SimpleFileOptions::default())
            .expect("zip entry");
        writer.write_all(&[0x99; 200]).expect("entry bytes");
        writer.finish().expect("finish zip");
    }

    let dest = std::env::temp_dir().join(format!("dekan_test_dest_{test_id}"));
    let strict_limits = ExtractLimits {
        max_single_file_bytes: 50,
        ..ExtractLimits::default()
    };

    let result = safe_extract_zip(Cursor::new(&archive_bytes), &dest, &strict_limits);
    assert!(result.is_err());

    assert_eq!(
        std::fs::read(&victim_file).expect("victim still exists"),
        b"IMMUTABLE_USER_DATA"
    );

    let _ = std::fs::remove_dir_all(&dest); // ignore-ok: cleanup test fixture
    let _ = std::fs::remove_dir_all(&outside_dir); // ignore-ok: cleanup test fixture
}

#[derive(Default, Debug)]
struct ConcurrentMoveState {
    attempted: bool,
    moved: bool,
    max_bytes: usize,
    rename_error: Option<String>,
}

struct ConcurrentMovingReader {
    cursor: Cursor<Vec<u8>>,
    target: std::path::PathBuf,
    moved: std::path::PathBuf,
    state: std::sync::Arc<std::sync::Mutex<ConcurrentMoveState>>,
    directory: bool,
}

impl std::io::Read for ConcurrentMovingReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut s = self.state.lock().expect("lock state");
        if !s.attempted && self.target.is_file() {
            s.attempted = true;
            let from = if self.directory {
                self.target.parent().expect("parent").to_path_buf()
            } else {
                self.target.clone()
            };
            let to = if self.directory {
                self.moved.parent().expect("parent").to_path_buf()
            } else {
                self.moved.clone()
            };
            let rename_res = std::thread::spawn(move || std::fs::rename(from, to))
                .join()
                .expect("join thread");
            match rename_res {
                Ok(()) => {
                    s.moved = true;
                    if !self.directory {
                        std::fs::write(&self.target, b"SENTINEL")?;
                    }
                }
                Err(e) => s.rename_error = Some(e.to_string()),
            }
        }
        if self.moved.is_file() {
            s.max_bytes = s
                .max_bytes
                .max(std::fs::metadata(&self.moved)?.len() as usize);
        }
        drop(s);
        self.cursor.read(buf)
    }
}

impl std::io::Seek for ConcurrentMovingReader {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        self.cursor.seek(pos)
    }
}

fn assert_concurrent_move_zero_external_writes(directory: bool) {
    let test_id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("epoch")
        .as_nanos();
    let base = std::env::temp_dir().join(format!(
        "dekan_zip_containment_{directory}_{}_{test_id}",
        std::process::id()
    ));
    let root = base.join("root");
    let outside = base.join("outside");
    std::fs::create_dir_all(&outside).expect("create outside");

    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .start_file(
            "sub/payload.bin",
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored),
        )
        .expect("start file");
    writer.write_all(&vec![0x5a; 262_144]).expect("write zip");
    let bytes = writer.finish().expect("finish zip").into_inner();

    let target = root.join("sub/payload.bin");
    let moved = if directory {
        outside.join("moved/payload.bin")
    } else {
        outside.join("payload.bin")
    };
    let state = std::sync::Arc::new(std::sync::Mutex::new(ConcurrentMoveState::default()));
    let reader = ConcurrentMovingReader {
        cursor: Cursor::new(bytes),
        target,
        moved: moved.clone(),
        state: state.clone(),
        directory,
    };

    let _ = safe_extract_zip(reader, &root, &ExtractLimits::default()); // ignore-ok: outcome checked via containment invariants
    let captured = state.lock().expect("lock state");
    assert!(captured.attempted, "mutation callback did not execute");
    assert_eq!(
        captured.max_bytes, 0,
        "writes reached an object moved outside trusted root"
    );
    assert!(
        !moved.exists(),
        "no extracted file may remain outside trusted root"
    );

    let _ = std::fs::remove_dir_all(&base); // ignore-ok: cleanup test fixture
}

#[test]
fn zip_file_move_contains_writes_and_preserves_cleanup_identity() {
    assert_concurrent_move_zero_external_writes(false);
}

#[test]
fn zip_directory_move_contains_writes() {
    assert_concurrent_move_zero_external_writes(true);
}
