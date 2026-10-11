use super::*;
use crate::trigger::prepare::mods::*;
use crate::trigger::prepare::paths::*;

#[test]
fn test_a_half_extracted_mod_directory_is_rebuilt_not_trusted() {
    let root = std::env::temp_dir().join(format!("dekan_mod_reextract_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root); // ignore-ok: fixture may not exist yet

    std::fs::create_dir_all(&root).expect("fixture root");
    let archive_path = root.join("81069.fantome");
    {
        let file = std::fs::File::create(&archive_path).expect("archive");
        let mut writer = zip::ZipWriter::new(file);
        let options: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
        writer.start_file("META/info.json", options).expect("meta");
        std::io::Write::write_all(&mut writer, br#"{"Name":"fixture"}"#).expect("meta body");
        writer
            .start_file("WAD/Ezreal.wad.client", options)
            .expect("wad");
        std::io::Write::write_all(&mut writer, b"RW\x03\x04").expect("wad body");
        writer.finish().expect("finish archive");
    }

    let target = root.join("81_81069");
    std::fs::create_dir_all(target.join("META")).expect("partial meta");
    assert!(
        !extracted_mod_is_complete(&target),
        "a directory with no WAD must not count as extracted"
    );

    prepare_mod_directory(&archive_path, &target).expect("re-extraction");

    assert!(
        extracted_mod_is_complete(&target),
        "the mod must be re-extracted rather than trusted for existing"
    );
    assert!(target.join("WAD").join("Ezreal.wad.client").is_file());

    let _ = std::fs::remove_dir_all(&root); // ignore-ok: fixture cleanup
}

#[test]
fn test_resolved_paths_discovery_in_an_empty_profile() {
    let root = std::env::temp_dir().join(format!(
        "dekan_discover_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&root); // ignore-ok: fixture cleanup
    let data = root.join("Dekan");
    let state = data.join("state");

    let paths = ResolvedPaths::discover_in(state.clone(), data.clone());
    assert_eq!(paths.state_dir, state);
    assert_eq!(paths.overlay_dir, data.join("overlay"));
    assert_eq!(paths.mods_dir, data.join("mods"));
    for category in ["skins", "maps", "fonts", "ui", "others"] {
        assert!(
            data.join("custom_mods").join(category).is_dir(),
            "custom mods category {category} created"
        );
    }
    let _ = std::fs::remove_dir_all(&root); // ignore-ok: the fixture may not exist yet
}

fn tools_fixture(tag: &str, files: &[&str]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "dekan_tools_{tag}_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: the fixture may not exist yet
    std::fs::create_dir_all(&dir).expect("fixture dir");
    for file in files {
        std::fs::write(dir.join(file), b"fixture").expect("fixture file");
    }
    dir
}

const BOTH_TOOLS: [&str; 2] = ["ltk_patcher_host.exe", "ltk_patcher_dll.dll"];

#[test]
fn test_first_candidate_folder_wins() {
    let first = tools_fixture("first", &BOTH_TOOLS);
    let second = tools_fixture("second", &BOTH_TOOLS);

    let (resolved, source) = resolve_tools_dir(&[first.clone(), second.clone()]);
    assert_eq!(resolved, first);
    assert_eq!(source, ToolsSource::Own);

    let _ = std::fs::remove_dir_all(&first); // ignore-ok: test temp dir teardown
    let _ = std::fs::remove_dir_all(&second); // ignore-ok: test temp dir teardown
}

#[test]
fn test_half_a_toolset_is_not_a_toolset() {
    let partial = tools_fixture("partial", &["ltk_patcher_host.exe"]);
    let (_, source) = resolve_tools_dir(std::slice::from_ref(&partial));
    assert_eq!(source, ToolsSource::Missing);
    let _ = std::fs::remove_dir_all(&partial); // ignore-ok: test temp dir teardown
}

#[test]
fn test_the_ltk_backend_is_the_toolset_and_another_injector_is_not() {
    let ltk = tools_fixture("ltk_only", &["ltk_patcher_host.exe", "ltk_patcher_dll.dll"]);
    let other = tools_fixture("other_only", &["other-injector.dll"]);
    assert_eq!(
        resolve_tools_dir(std::slice::from_ref(&ltk)).1,
        ToolsSource::Own
    );
    assert_eq!(
        resolve_tools_dir(std::slice::from_ref(&other)).1,
        ToolsSource::Missing
    );
    let _ = std::fs::remove_dir_all(&ltk); // ignore-ok: test temp dir teardown
    let _ = std::fs::remove_dir_all(&other); // ignore-ok: test temp dir teardown
}

#[test]
fn test_missing_tools_point_at_our_own_folder() {
    let absent = std::env::temp_dir().join("dekan_tools_absent_does_not_exist");
    let own = PathBuf::from(r"C:\Program Files\Dekan\tools");
    let (resolved, source) = resolve_tools_dir(&[own.clone(), absent]);

    if source == ToolsSource::Missing {
        assert_eq!(resolved, own);
    }
}

#[test]
fn test_game_dir_normalization_distinguishes_client_root() {
    let temp = std::env::temp_dir().join(format!("dekan_game_norm_test_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp); // ignore-ok: cleanup

    let client_root = temp.join("League of Legends");
    std::fs::create_dir_all(client_root.join("DATA")).expect("client DATA");
    std::fs::write(client_root.join("LeagueClientUx.exe"), b"mock client").expect("client exe");

    let game_dir = client_root.join("Game");
    std::fs::create_dir_all(game_dir.join("DATA")).expect("game DATA");
    std::fs::write(game_dir.join("League of Legends.exe"), b"mock game").expect("game exe");

    let normalized = dekan_platform::paths::normalize_game_dir(&client_root);
    assert_eq!(normalized, Some(game_dir.clone()));

    let normalized_game = dekan_platform::paths::normalize_game_dir(&game_dir);
    assert_eq!(normalized_game, Some(game_dir));

    let _ = std::fs::remove_dir_all(&temp); // ignore-ok: cleanup
}

fn shared_skin_link(slots: &[u32]) -> String {
    let mut parts: Vec<String> = slots.iter().map(|n| format!("Skins_Skin{n}")).collect();
    parts.sort();
    format!(
        "DATA/Characters/Zed/Zed_Multi_Skins_{}.bin",
        parts.join("_")
    )
}

fn write_test_wad(path: &std::path::Path, entries: &[(u64, Vec<u8>)]) {
    let mut writer = dekan_wad::WadWriter::default();
    for (hash, bytes) in entries {
        writer.insert(
            *hash,
            dekan_wad::writer::optimal_raw(bytes.clone()).expect("entry"),
        );
    }
    std::fs::create_dir_all(path.parent().expect("parent")).expect("dir");
    std::fs::write(path, writer.to_bytes().expect("wad")).expect("wad");
}

fn staged_mod(mods_dir: &std::path::Path, name: &str, links: &[String]) {
    write_test_wad(
        &mods_dir.join(name).join("WAD").join("Zed.wad.client"),
        &[(
            dekan_wad::hash::wad_path_hash("data/characters/zed/skins/skin0.bin"),
            dekan_wad::prop::serialize_prop_links(links, 3),
        )],
    );
}

#[test]
fn test_at_injection_a_renamed_link_is_repaired_and_a_mod_without_successor_is_dropped() {
    let root = std::env::temp_dir().join(format!(
        "dekan_drop_incompatible_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&root); // ignore-ok: fixture may not exist yet
    let game_dir = root.join("Game");
    let current = shared_skin_link(&[0, 1, 44]);
    write_test_wad(
        &game_dir
            .join("DATA")
            .join("FINAL")
            .join("Champions")
            .join("Zed.wad.client"),
        &[
            (
                dekan_wad::hash::wad_path_hash("data/characters/zed/skins/skin0.bin"),
                dekan_wad::prop::serialize_prop_links(std::slice::from_ref(&current), 3),
            ),
            (
                dekan_wad::hash::wad_path_hash(&current.to_ascii_lowercase()),
                b"shared".to_vec(),
            ),
        ],
    );
    let mods_dir = root.join("mods");
    staged_mod(&mods_dir, "cm_renamed", &[shared_skin_link(&[0, 1])]);
    staged_mod(&mods_dir, "cm_clean", std::slice::from_ref(&current));
    staged_mod(&mods_dir, "cm_lost", &[shared_skin_link(&[0, 5])]);

    let kept = drop_incompatible_mods(
        vec!["cm_renamed".into(), "cm_clean".into(), "cm_lost".into()],
        &mods_dir,
        &game_dir,
    );
    assert_eq!(kept, vec!["cm_renamed".to_owned(), "cm_clean".to_owned()]);
    let repaired = dekan_wad::WadFile::open(
        &mods_dir
            .join("cm_renamed")
            .join("WAD")
            .join("Zed.wad.client"),
    )
    .expect("open")
    .read(dekan_wad::hash::wad_path_hash(
        "data/characters/zed/skins/skin0.bin",
    ))
    .expect("read")
    .expect("skin0");
    assert_eq!(
        dekan_wad::prop::parse_prop_links(&repaired).expect("links"),
        vec![current]
    );
    let _ = std::fs::remove_dir_all(&root); // ignore-ok: cleanup
}

#[test]
fn test_at_injection_an_unreadable_wad_is_kept_and_a_wad_that_cannot_be_rewritten_is_dropped() {
    let root = std::env::temp_dir().join(format!(
        "dekan_drop_errors_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&root); // ignore-ok: fixture may not exist yet
    let game_dir = root.join("Game");
    let current = shared_skin_link(&[0, 1, 44]);
    write_test_wad(
        &game_dir
            .join("DATA")
            .join("FINAL")
            .join("Champions")
            .join("Zed.wad.client"),
        &[
            (
                dekan_wad::hash::wad_path_hash("data/characters/zed/skins/skin0.bin"),
                dekan_wad::prop::serialize_prop_links(std::slice::from_ref(&current), 3),
            ),
            (
                dekan_wad::hash::wad_path_hash(&current.to_ascii_lowercase()),
                b"shared".to_vec(),
            ),
        ],
    );
    let mods_dir = root.join("mods");
    let broken = mods_dir
        .join("cm_broken")
        .join("WAD")
        .join("Zed.wad.client");
    std::fs::create_dir_all(broken.parent().expect("parent")).expect("dir");
    std::fs::write(&broken, b"not a wad").expect("garbage");
    staged_mod(&mods_dir, "cm_locked", &[shared_skin_link(&[0, 1])]);
    let locked = mods_dir
        .join("cm_locked")
        .join("WAD")
        .join("Zed.wad.client");
    let mut permissions = std::fs::metadata(&locked).expect("meta").permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&locked, permissions.clone()).expect("readonly");

    let kept = drop_incompatible_mods(
        vec!["cm_broken".into(), "cm_locked".into()],
        &mods_dir,
        &game_dir,
    );
    assert_eq!(
        kept,
        vec!["cm_broken".to_owned()],
        "an unread WAD is no evidence of damage; a dangling link that could not be fixed is"
    );
    #[allow(clippy::permissions_set_readonly_false)]
    permissions.set_readonly(false);
    std::fs::set_permissions(&locked, permissions).expect("writable");
    let _ = std::fs::remove_dir_all(&root); // ignore-ok: cleanup
}
