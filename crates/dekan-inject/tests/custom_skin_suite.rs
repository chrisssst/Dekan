use std::path::Path;
use std::sync::atomic::AtomicBool;

use dekan_inject::overlay_builder;
use dekan_wad::WadFile;

fn create_synthetic_game_wad(path: &Path, hashes: &[u64]) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("game wad parent");
    }
    let entry_count = hashes.len() as u32;
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

    for (i, &hash) in hashes.iter().enumerate() {
        data.extend_from_slice(&hash.to_le_bytes());
        let offset = data_offset + (i as u32 * 12);
        data.extend_from_slice(&offset.to_le_bytes());
        data.extend_from_slice(&12u32.to_le_bytes());
        data.extend_from_slice(&12u32.to_le_bytes());
        data.push(0);
        data.push(0);
        data.extend_from_slice(&[0u8; 2]);
        data.extend_from_slice(&[0u8; 8]);
    }

    for _ in 0..entry_count {
        data.extend_from_slice(b"GAME_BASE_OK");
    }

    std::fs::write(path, data).expect("write game wad");
}

fn create_synthetic_mod(mod_dir: &Path, mount_name: &str, mod_hash: u64, mod_payload: &[u8]) {
    let meta_dir = mod_dir.join("META");
    std::fs::create_dir_all(&meta_dir).expect("meta dir");
    let info = serde_json::json!({
        "Name": "Blood Moon Aatrox Custom",
        "Author": "CommunityArtist",
        "Version": "2.0.0"
    });
    std::fs::write(
        meta_dir.join("info.json"),
        serde_json::to_vec(&info).expect("json"),
    )
    .expect("write info");

    let wad_dir = mod_dir.join("WAD").join(mount_name);
    std::fs::create_dir_all(&wad_dir).expect("mod wad dir");

    let entry_file = wad_dir.join(format!("{mod_hash:016x}.bin"));
    std::fs::write(entry_file, mod_payload).expect("write mod entry");
}

#[test]
fn test_custom_skin_overlay_builder_full_lifecycle() {
    let test_id = format!("dekan_overlay_test_{}", std::process::id());
    let base = std::env::temp_dir().join(test_id);
    let game_dir = base.join("Game");
    let mods_dir = base.join("mods");
    let overlay_dir = base.join("overlay");

    let _ = std::fs::remove_dir_all(&base); // ignore-ok: cleanup previous run
    std::fs::create_dir_all(&game_dir).expect("game dir");
    std::fs::create_dir_all(&mods_dir).expect("mods dir");
    std::fs::create_dir_all(&overlay_dir).expect("overlay dir");

    let champ_wad = game_dir
        .join("DATA")
        .join("FINAL")
        .join("Champions")
        .join("Aatrox.wad.client");
    create_synthetic_game_wad(&champ_wad, &[0x1111_2222_3333_4444, 0x5555_6666_7777_8888]);

    let mod_name = "AatroxBloodMoon";
    let custom_mod_dir = mods_dir.join(mod_name);
    create_synthetic_mod(
        &custom_mod_dir,
        "Aatrox.wad.client",
        0x1111_2222_3333_4444,
        b"CUSTOM_SKIN_PAYLOAD_DATA",
    );

    let cancel = AtomicBool::new(false);
    let build_result = overlay_builder::build(
        &game_dir,
        &mods_dir,
        &overlay_dir,
        &[mod_name.to_string()],
        &cancel,
    );

    assert!(build_result.is_ok(), "build overlay should succeed");
    let build = build_result.expect("build result");
    assert_eq!(build.wad_files, 1);
    assert!(build.written > 0);
    assert!(build.bytes > 0);

    let generated_overlay_wad = overlay_dir
        .join("DATA")
        .join("FINAL")
        .join("Champions")
        .join("Aatrox.wad.client");
    assert!(
        generated_overlay_wad.is_file(),
        "overlay wad file must be generated"
    );

    let opened = WadFile::open(&generated_overlay_wad).expect("open generated overlay");
    assert_eq!(opened.len(), 2);

    let read_mod_entry = opened
        .read(0x1111_2222_3333_4444)
        .expect("read modified entry");
    assert!(read_mod_entry.is_some());
    assert_eq!(
        read_mod_entry.expect("bytes"),
        b"CUSTOM_SKIN_PAYLOAD_DATA",
        "custom skin payload must replace the stock game asset"
    );

    let read_stock_entry = opened
        .read(0x5555_6666_7777_8888)
        .expect("read stock entry");
    assert!(read_stock_entry.is_some());
    assert_eq!(
        read_stock_entry.expect("bytes"),
        b"GAME_BASE_OK",
        "untouched game asset must be preserved"
    );

    let _ = std::fs::remove_dir_all(&base); // ignore-ok: cleanup fixture
}

#[test]
fn test_custom_skin_overlay_builder_cancellation_and_cleanup() {
    let test_id = format!("dekan_cancel_test_{}", std::process::id());
    let base = std::env::temp_dir().join(test_id);
    let game_dir = base.join("Game");
    let mods_dir = base.join("mods");
    let overlay_dir = base.join("overlay");

    let _ = std::fs::remove_dir_all(&base); // ignore-ok: cleanup previous run
    std::fs::create_dir_all(&game_dir).expect("game dir");
    std::fs::create_dir_all(&mods_dir).expect("mods dir");
    std::fs::create_dir_all(&overlay_dir).expect("overlay dir");

    let champ_wad = game_dir
        .join("DATA")
        .join("FINAL")
        .join("Champions")
        .join("Yasuo.wad.client");
    create_synthetic_game_wad(&champ_wad, &[0xAAAA_BBBB_CCCC_DDDD]);

    let cancel = AtomicBool::new(true);
    let build_result = overlay_builder::build(&game_dir, &mods_dir, &overlay_dir, &[], &cancel);

    assert!(build_result.is_err());
    assert!(matches!(
        build_result,
        Err(dekan_inject::error::InjectError::Cancelled)
    ));

    let _ = std::fs::remove_dir_all(&base); // ignore-ok: cleanup fixture
}
