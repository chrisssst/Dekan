use super::*;

fn write_lines(path: &Path, count: usize) {
    std::fs::create_dir_all(path.parent().expect("parent")).expect("dir");
    std::fs::write(path, "x\n".repeat(count)).expect("write");
}

#[test]
fn files_over_the_limit_are_reported_and_build_output_and_other_files_are_not() {
    let root = std::env::temp_dir().join(format!("dekan_file_size_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root); // ignore-ok: the folder may not exist yet
    write_lines(&root.join("crates/a/src/long.rs"), MAX_LINES + 1);
    write_lines(&root.join("crates/a/src/edge.rs"), MAX_LINES);
    write_lines(&root.join("crates/a/ui/long.slint"), MAX_LINES + 1);
    write_lines(&root.join("crates/target/debug/build.rs"), MAX_LINES * 2);
    write_lines(&root.join("crates/a/README.md"), MAX_LINES * 2);

    let (checked, offenders) = oversized_files(&root, &["crates", "missing"]);

    assert_eq!(checked, 3);
    assert_eq!(offenders.len(), 2, "{offenders:?}");
    assert!(offenders[0].contains("long.rs") && offenders[0].ends_with("401 lines"));
    assert!(offenders[1].contains("long.slint"));
    let _ = std::fs::remove_dir_all(&root); // ignore-ok: cleanup of the test folder
}

#[test]
fn folders_over_the_file_limit_are_reported() {
    let root = std::env::temp_dir().join(format!("dekan_folder_size_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root); // ignore-ok: the folder may not exist yet
    for i in 0..=MAX_FILES_PER_FOLDER {
        write_lines(&root.join(format!("crates/a/src/wide/m{i}.rs")), 1);
    }
    for i in 0..MAX_FILES_PER_FOLDER {
        write_lines(&root.join(format!("crates/a/src/full/m{i}.rs")), 1);
    }
    write_lines(&root.join("crates/a/src/full/notes.md"), 1);

    let (checked, offenders) = oversized_files(&root, &["crates"]);

    assert_eq!(checked, 2 * MAX_FILES_PER_FOLDER + 1);
    assert_eq!(offenders.len(), 1, "{offenders:?}");
    assert!(offenders[0].contains("wide") && offenders[0].ends_with("6 code files"));
    let _ = std::fs::remove_dir_all(&root); // ignore-ok: cleanup of the test folder
}
