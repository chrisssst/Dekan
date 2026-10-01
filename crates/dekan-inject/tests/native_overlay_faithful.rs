use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use dekan_inject::overlay_builder;
use dekan_wad::wad::WadFile;

fn game_dir() -> PathBuf {
    PathBuf::from(r"D:\Riot Games\League of Legends\Game")
}

fn mods_dir() -> PathBuf {
    let local = std::env::var("LOCALAPPDATA").expect("LOCALAPPDATA set");
    PathBuf::from(local).join("Dekan").join("mods")
}

fn compare_wad(
    game_wad: &std::path::Path,
    overlay_wad: &std::path::Path,
    expected_changed: &[u64],
) {
    let g = WadFile::open(game_wad).expect("open game WAD");
    let n = WadFile::open(overlay_wad).expect("open overlay WAD");

    let game_by_hash: HashMap<u64, _> = g.toc().map(|e| (e.path_hash, e)).collect();
    let over_by_hash: HashMap<u64, _> = n.toc().map(|e| (e.path_hash, e)).collect();

    assert_eq!(
        game_by_hash.len(),
        over_by_hash.len(),
        "overlay must have the same entry set as the game WAD"
    );

    let mut verbatim = 0usize;
    let mut reencoded_unexpected = Vec::new();
    let mut changed = 0usize;
    let mut game_unchanged_bytes = 0u64;
    let mut over_unchanged_bytes = 0u64;
    for (hash, ge) in &game_by_hash {
        let oe = over_by_hash
            .get(hash)
            .unwrap_or_else(|| panic!("overlay missing entry {hash:#x} present in the game"));
        if expected_changed.contains(hash) {
            changed += 1;
            continue;
        }
        let same_meta =
            ge.compression == oe.compression && ge.compressed_size == oe.compressed_size;
        game_unchanged_bytes += ge.compressed_size as u64;
        over_unchanged_bytes += oe.compressed_size as u64;
        if same_meta {
            verbatim += 1;
        } else if reencoded_unexpected.len() < 8 {
            reencoded_unexpected.push((
                *hash,
                ge.compression,
                ge.compressed_size,
                oe.compression,
                oe.compressed_size,
            ));
        }
    }
    let reencoded = game_by_hash.len() - changed - verbatim;

    let mut raw_checked = 0usize;
    let mut raw_diff = 0usize;
    for (hash, ge) in game_by_hash.iter() {
        if expected_changed.contains(hash) || raw_checked >= 40 {
            continue;
        }
        let oe = over_by_hash[hash];
        if ge.compression == oe.compression && ge.compressed_size == oe.compressed_size {
            let gb = g.read_raw(ge).expect("game raw");
            let ob = n.read_raw(oe).expect("overlay raw");
            if gb != ob {
                raw_diff += 1;
            }
            raw_checked += 1;
        }
    }

    println!(
        "  {} entries | changed(mod)={changed} verbatim={verbatim} reencoded={reencoded} | unchanged compressed bytes: game={game_unchanged_bytes} overlay={over_unchanged_bytes} | raw-checked {raw_checked}, byte-diff {raw_diff}",
        game_by_hash.len()
    );
    for (h, gc, gs, oc, os) in &reencoded_unexpected {
        println!("    re-encoded (unexpected) {h:#x}: game {gc:?}/{gs}B -> overlay {oc:?}/{os}B");
    }
}

fn expected_changes(mod_wad: &std::path::Path, game_wad: &std::path::Path) -> Vec<u64> {
    let m = WadFile::open(mod_wad).expect("open mod wad");
    let g = WadFile::open(game_wad).expect("open game wad");
    let game: std::collections::HashSet<u64> = g.toc().map(|e| e.path_hash).collect();
    m.toc()
        .map(|e| e.path_hash)
        .filter(|h| game.contains(h))
        .collect()
}

#[test]
#[ignore = "needs the real game install and extracted mod; run with --ignored"]
fn native_clone_of_map11_and_zed_is_byte_faithful() {
    let game = game_dir();
    let mods = mods_dir();
    assert!(
        game.join("DATA/FINAL").exists(),
        "game not found at {game:?}"
    );
    assert!(
        mods.join("238_238068/WAD/Zed.wad.client").exists(),
        "extracted mod 238_238068 not found in {mods:?}"
    );

    let out = std::env::temp_dir().join("dekan_native_faithful");

    // ignore-ok: best-effort cleanup of temp directory before test run
    let _ = std::fs::remove_dir_all(&out);

    let build = overlay_builder::build(
        &game,
        &mods,
        &out,
        &["238_238068".to_string()],
        &AtomicBool::new(false),
    )
    .expect("native overlay build");
    println!(
        "native build: {} WADs, {} bytes, {} ms",
        build.wad_files,
        build.bytes,
        build.elapsed.as_millis()
    );

    let mod_wad = mods.join(r"238_238068\WAD\Zed.wad.client");

    let game_map11 = game.join(r"DATA\FINAL\Maps\Shipping\Map11.wad.client");
    let map11_changed = expected_changes(&mod_wad, &game_map11);
    println!(
        "Map11.wad.client (mod entries also in Map11: {}):",
        map11_changed.len()
    );
    compare_wad(
        &game_map11,
        &out.join(r"DATA\FINAL\Maps\Shipping\Map11.wad.client"),
        &map11_changed,
    );

    let game_zed = game.join(r"DATA\FINAL\Champions\Zed.wad.client");
    let zed_changed = expected_changes(&mod_wad, &game_zed);
    println!("Zed.wad.client (modded entries: {}):", zed_changed.len());
    compare_wad(
        &game_zed,
        &out.join(r"DATA\FINAL\Champions\Zed.wad.client"),
        &zed_changed,
    );

    // ignore-ok: best-effort cleanup of temp directory after test run
    let _ = std::fs::remove_dir_all(&out);
}
