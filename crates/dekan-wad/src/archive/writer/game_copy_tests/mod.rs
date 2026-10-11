use super::tests::{memory, temp_dir};
use super::*;
use crate::wad::WadFile;

fn game_wad(dir: &Path) -> PathBuf {
    let mut game = WadWriter::new([7u8; WAD_SIGNATURE_SIZE]);
    game.insert(0xA, memory(0, b"model", 5));
    game.insert(0xB, memory(0, b"shadow skin", 11));
    game.insert(0xC, memory(0, b"terrain", 7));
    let path = dir.join("Map11.wad.client");
    let mut bytes = game.to_bytes().expect("game");
    bytes[260..268].copy_from_slice(&0xABCD_EF01_2345_6789u64.to_le_bytes());
    std::fs::write(&path, bytes).expect("write game");
    path
}

fn rebased(game_path: &Path, replacements: &[(u64, &[u8])]) -> WadWriter {
    let source = WadFile::open(game_path).expect("open");
    let mut writer = WadWriter::rebased_on(&source);
    let index = writer.add_source(game_path);
    for entry in source.toc() {
        writer.insert(entry.path_hash, WriterEntry::from_wad(index, entry));
    }
    for (name, bytes) in replacements {
        writer.insert(*name, memory(0, bytes, bytes.len() as u64));
    }
    writer
}

#[test]
fn test_a_replacement_only_wad_is_the_game_copy_plus_its_new_entries() {
    let dir = temp_dir("over_copy");
    let game_path = game_wad(&dir);
    let game_bytes = std::fs::read(&game_path).expect("game bytes");
    let out = dir.join("out").join("Map11.wad.client");

    let outcome = rebased(&game_path, &[(0xB, b"new shadow skin")])
        .write_over_game_copy(&out, "4", &|| false)
        .expect("write")
        .expect("eligible");
    assert!(matches!(outcome, WriteOutcome::Written { .. }));

    let written = std::fs::read(&out).expect("overlay");
    assert_eq!(
        &written[..272],
        &game_bytes[..272],
        "the game's header, untouched"
    );
    let data_start = 272 + 3 * WAD_ENTRY_SIZE;
    assert_eq!(
        &written[data_start..game_bytes.len()],
        &game_bytes[data_start..],
        "every game byte stays where the game has it"
    );
    let wad = WadFile::open(&out).expect("reopen");
    assert_eq!(
        wad.read(0xB).expect("b").as_deref(),
        Some(&b"new shadow skin"[..])
    );
    assert_eq!(wad.read(0xA).expect("a").as_deref(), Some(&b"model"[..]));
    assert_eq!(wad.read(0xC).expect("c").as_deref(), Some(&b"terrain"[..]));
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: fixture cleanup
}

#[test]
fn test_the_game_copy_is_reused_until_the_game_file_changes() {
    let dir = temp_dir("over_copy_reuse");
    let game_path = game_wad(&dir);
    let out = dir.join("out").join("Map11.wad.client");
    rebased(&game_path, &[(0xB, b"first skin")])
        .write_over_game_copy(&out, "4", &|| false)
        .expect("first")
        .expect("eligible");

    let mut marker = std::fs::OpenOptions::new()
        .write(true)
        .open(&out)
        .expect("open copy");
    marker
        .seek(SeekFrom::Start(272 + 3 * WAD_ENTRY_SIZE as u64 + 1))
        .expect("seek");
    marker.write_all(b"Z").expect("mark the copy");
    drop(marker);

    rebased(&game_path, &[(0xB, b"second")])
        .write_over_game_copy(&out, "4", &|| false)
        .expect("second")
        .expect("eligible");
    let reused = std::fs::read(&out).expect("overlay");
    assert_eq!(
        reused[272 + 3 * WAD_ENTRY_SIZE + 1],
        b'Z',
        "the copy was not made again"
    );
    assert_eq!(
        WadFile::open(&out)
            .expect("reopen")
            .read(0xB)
            .expect("b")
            .as_deref(),
        Some(&b"second"[..]),
        "the old tail is cut and the new entry lands"
    );

    let mut game = std::fs::read(&game_path).expect("game");
    game.push(0);
    std::fs::write(&game_path, game).expect("patched game");
    rebased(&game_path, &[(0xB, b"third")])
        .write_over_game_copy(&out, "4", &|| false)
        .expect("third")
        .expect("eligible");
    assert_ne!(
        std::fs::read(&out).expect("overlay")[272 + 3 * WAD_ENTRY_SIZE + 1],
        b'Z',
        "a changed game file is copied again"
    );
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: fixture cleanup
}

#[test]
fn test_a_mod_that_adds_entries_is_not_written_over_a_copy() {
    let dir = temp_dir("over_copy_added");
    let game_path = game_wad(&dir);
    let out = dir.join("out").join("Map11.wad.client");
    let outcome = rebased(&game_path, &[(0xD, b"brand new")])
        .write_over_game_copy(&out, "4", &|| false)
        .expect("checked");
    assert_eq!(outcome, None);
    assert!(!out.exists());
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: fixture cleanup
}
