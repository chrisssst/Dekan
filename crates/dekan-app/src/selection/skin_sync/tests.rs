use super::*;

#[test]
fn test_sync_is_off_unless_a_repository_is_named() {
    for off in [
        None,
        Some(""),
        Some("1"),
        Some("true"),
        Some("owner"),
        Some("/repo"),
    ] {
        assert!(SkinSyncConfig::from_env_value(off).is_none(), "{off:?}");
    }
    for bad in [
        "../repo",
        "owner/..",
        "own er/repo",
        "owner/re/po",
        "owner/repo?x=1",
    ] {
        assert!(SkinSyncConfig::from_env_value(Some(bad)).is_none(), "{bad}");
    }
    let config = SkinSyncConfig::from_env_value(Some(" someone/skin-library ")).expect("valid");
    assert_eq!(
        config.api_base,
        "https://api.github.com/repos/someone/skin-library"
    );
    assert_eq!(
        config.zip_url,
        "https://github.com/someone/skin-library/archive/refs/heads/main.zip"
    );
}
use std::io::Write;
use std::path::PathBuf;
use zip::write::SimpleFileOptions;

fn temp_test_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("dekan_test_skins_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: best effort test cleanup
    let _ = std::fs::create_dir_all(&dir); // ignore-ok: test setup
    dir
}

#[test]
fn test_dir_has_skins_detects_numeric_folders() {
    let temp = temp_test_dir("numeric");
    assert!(!dir_has_skins(&temp));

    std::fs::write(temp.join(".skin_version"), "abc123").unwrap();
    assert!(!dir_has_skins(&temp));

    std::fs::create_dir(temp.join("238")).unwrap();
    assert!(dir_has_skins(&temp));

    let _ = std::fs::remove_dir_all(&temp); // ignore-ok: cleanup
}

#[test]
fn test_local_sha_roundtrip() {
    let temp = temp_test_dir("sha");
    assert_eq!(get_local_sha(&temp), None);

    save_local_sha(&temp, "deadbeef12345678").unwrap();
    assert_eq!(get_local_sha(&temp), Some("deadbeef12345678".to_string()));

    let _ = std::fs::remove_dir_all(&temp); // ignore-ok: cleanup
}

#[test]
fn test_safe_extraction_strips_repo_prefix() {
    let temp = temp_test_dir("extract");
    let zip_path = temp.join("test_skins.zip");

    {
        let file = std::fs::File::create(&zip_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = SimpleFileOptions::default();

        zip.start_file("skin-library-main/skins/238/238001/238001.fantome", options)
            .unwrap();
        zip.write_all(b"mock_skin_content").unwrap();

        zip.finish().unwrap();
    }

    let zip_bytes = std::fs::read(&zip_path).unwrap();
    let cursor = Cursor::new(zip_bytes);
    let mut archive = ZipArchive::new(cursor).unwrap();
    let target_dir = temp.join("library");
    std::fs::create_dir_all(&target_dir).unwrap();

    let _limits = ExtractLimits::default();
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).unwrap();
        let raw_name = entry.name().unwrap().into_owned();
        if let Some(idx) = raw_name.find("/skins/") {
            let rel = &raw_name[idx + "/skins/".len()..];
            let validated = validate_archive_path(Path::new(rel)).unwrap();
            let dest = target_dir.join(&validated);
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf).unwrap();
            atomic_write(&dest, &buf, false).unwrap();
        }
    }

    let extracted_file = target_dir.join("238").join("238001").join("238001.fantome");
    assert!(extracted_file.is_file());
    assert_eq!(std::fs::read(extracted_file).unwrap(), b"mock_skin_content");

    let _ = std::fs::remove_dir_all(&temp); // ignore-ok: cleanup
}
