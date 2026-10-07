use super::*;

fn create_dummy_wad(path: &Path, content_byte: u8, len: usize) {
    let parent = path.parent().unwrap();
    std::fs::create_dir_all(parent).unwrap();
    let mut data = vec![content_byte; len.max(WAD_HEADER_SIZE)];
    data[0..4].copy_from_slice(b"RW\x03\x04");
    std::fs::write(path, data).unwrap();
}

#[test]
fn test_cache_hit_and_invalidation_on_game_patch() {
    let temp = std::env::temp_dir().join(format!("dekan_cache_test_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp); // ignore-ok: cleanup old test dir

    let game_dir = temp.join("Game");
    let mods_dir = temp.join("mods");
    let overlay_dir = temp.join("overlay");

    let mod_a = mods_dir.join("mod_zed");
    std::fs::create_dir_all(&mod_a).unwrap();
    std::fs::write(mod_a.join("info.json"), "{}").unwrap();

    let game_wad = game_dir.join("DATA/FINAL/Champions/Zed.wad.client");
    create_dummy_wad(&game_wad, 0x11, 1024);

    let overlay_wad = overlay_dir.join("DATA/FINAL/Champions/Zed.wad.client");
    create_dummy_wad(&overlay_wad, 0x22, 2048);

    let mods = vec!["mod_zed".to_string()];

    assert_eq!(
        OverlayCache::is_fresh(&game_dir, &mods_dir, &overlay_dir, &mods),
        None
    );

    OverlayCache::record(&game_dir, &mods_dir, &overlay_dir, &mods).expect("record ok");

    let hit = OverlayCache::is_fresh(&game_dir, &mods_dir, &overlay_dir, &mods);
    assert_eq!(hit, Some((1, 2048)));

    create_dummy_wad(&game_wad, 0x33, 1024);
    assert_eq!(
        OverlayCache::is_fresh(&game_dir, &mods_dir, &overlay_dir, &mods),
        None
    );

    OverlayCache::record(&game_dir, &mods_dir, &overlay_dir, &mods).expect("record ok");
    assert_eq!(
        OverlayCache::is_fresh(&game_dir, &mods_dir, &overlay_dir, &mods),
        Some((1, 2048))
    );

    OverlayCache::invalidate(&overlay_dir);
    assert_eq!(
        OverlayCache::is_fresh(&game_dir, &mods_dir, &overlay_dir, &mods),
        None
    );

    let _ = std::fs::remove_dir_all(&temp); // ignore-ok: cleanup
}

#[test]
fn test_cache_miss_on_mod_file_change() {
    let temp = std::env::temp_dir().join(format!("dekan_cache_mod_change_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp); // ignore-ok: cleanup old test dir

    let game_dir = temp.join("Game");
    let mods_dir = temp.join("mods");
    let overlay_dir = temp.join("overlay");

    let mod_a = mods_dir.join("mod_nasus");
    std::fs::create_dir_all(&mod_a).unwrap();
    let file_path = mod_a.join("file.bin");
    std::fs::write(&file_path, "v1").unwrap();

    let overlay_wad = overlay_dir.join("DATA/FINAL/Champions/Nasus.wad.client");
    create_dummy_wad(&overlay_wad, 0x44, 4096);

    let mods = vec!["mod_nasus".to_string()];
    OverlayCache::record(&game_dir, &mods_dir, &overlay_dir, &mods).expect("record");
    assert_eq!(
        OverlayCache::is_fresh(&game_dir, &mods_dir, &overlay_dir, &mods),
        Some((1, 4096))
    );

    std::fs::write(&file_path, "v2-modified-larger-content").unwrap();
    assert_eq!(
        OverlayCache::is_fresh(&game_dir, &mods_dir, &overlay_dir, &mods),
        None
    );

    let _ = std::fs::remove_dir_all(&temp); // ignore-ok: cleanup
}

struct Recorded {
    root: PathBuf,
    game_dir: PathBuf,
    mods_dir: PathBuf,
    overlay_dir: PathBuf,
    mods: Vec<String>,
}

impl Recorded {
    fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!("dekan_cache_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root); // ignore-ok: cleanup old test dir
        let (game_dir, mods_dir, overlay_dir) =
            (root.join("Game"), root.join("mods"), root.join("overlay"));
        let mod_dir = mods_dir.join("mod_ahri");
        std::fs::create_dir_all(&mod_dir).unwrap();
        std::fs::write(mod_dir.join("info.json"), "{}").unwrap();
        create_dummy_wad(
            &overlay_dir.join("DATA/FINAL/Champions/Ahri.wad.client"),
            0x55,
            1024,
        );
        let mods = vec!["mod_ahri".to_string()];
        OverlayCache::record(&game_dir, &mods_dir, &overlay_dir, &mods).expect("record");
        Self {
            root,
            game_dir,
            mods_dir,
            overlay_dir,
            mods,
        }
    }

    fn is_fresh(&self) -> Option<(usize, u64)> {
        OverlayCache::is_fresh(
            &self.game_dir,
            &self.mods_dir,
            &self.overlay_dir,
            &self.mods,
        )
    }

    fn edit_fingerprint(&self, edit: impl FnOnce(&mut serde_json::Value)) {
        let path = self.overlay_dir.join(FINGERPRINT_FILE);
        let mut json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        edit(&mut json);
        std::fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
    }
}

impl Drop for Recorded {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root); // ignore-ok: test cleanup
    }
}

#[test]
fn test_cache_miss_when_the_builder_revision_changes() {
    let cache = Recorded::new("revision_change");
    assert_eq!(cache.is_fresh(), Some((1, 1024)));

    cache.edit_fingerprint(|fp| {
        fp["builder_revision"] = serde_json::json!(OVERLAY_BUILDER_REVISION + 1);
    });
    assert_eq!(cache.is_fresh(), None);
}

#[test]
fn test_cache_miss_on_a_v1_fingerprint() {
    let cache = Recorded::new("v1_fingerprint");
    assert_eq!(cache.is_fresh(), Some((1, 1024)));

    cache.edit_fingerprint(|fp| {
        let object = fp.as_object_mut().unwrap();
        object.remove("builder");
        object.remove("builder_revision");
        object.insert("version".into(), serde_json::json!(1));
    });
    assert_eq!(cache.is_fresh(), None);

    let tagged = Recorded::new("v1_tagged");
    tagged.edit_fingerprint(|fp| fp["version"] = serde_json::json!(1));
    assert_eq!(tagged.is_fresh(), None);
}
