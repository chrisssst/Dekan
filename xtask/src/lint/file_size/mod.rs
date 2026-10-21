use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub(crate) const MAX_LINES: usize = 400;
pub(crate) const MAX_FILES_PER_FOLDER: usize = 5;

const CODE_EXTENSIONS: [&str; 2] = ["rs", "slint"];

pub(crate) fn run_file_size_check() {
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf();

    let (checked, offenders) = oversized_files(&workspace_root, &["crates", "xtask"]);
    if checked == 0 {
        eprintln!(
            "
[ERROR] File size sweep: no code file found under crates or xtask"
        );
        std::process::exit(1);
    }
    if offenders.is_empty() {
        println!(
            "  {checked} code files, none over {MAX_LINES} lines, no folder over {MAX_FILES_PER_FOLDER}"
        );
        return;
    }

    eprintln!(
        "\n[ERROR] File size sweep: {} file(s) over {MAX_LINES} lines or folder(s) over {MAX_FILES_PER_FOLDER} code files; split them by responsibility:",
        offenders.len()
    );
    for offender in &offenders {
        eprintln!("  {offender}");
    }
    std::process::exit(1);
}

pub(crate) fn oversized_files(root: &Path, dirs: &[&str]) -> (usize, Vec<String>) {
    let mut checked = 0usize;
    let mut offenders = Vec::new();
    let mut folders: BTreeMap<PathBuf, usize> = BTreeMap::new();
    let mut unreadable = Vec::new();
    for dir in dirs {
        collect_code_files(&root.join(dir), &mut unreadable, &mut |path| {
            checked += 1;
            if let Some(folder) = path.parent() {
                *folders.entry(folder.to_path_buf()).or_default() += 1;
            }
            match std::fs::read_to_string(path) {
                Ok(content) => {
                    let lines = content.lines().count();
                    if lines > MAX_LINES {
                        offenders.push(format!("{}: {lines} lines", path.display()));
                    }
                }
                Err(e) => offenders.push(format!("{}: unreadable ({e})", path.display())),
            }
        });
    }
    offenders.append(&mut unreadable);
    for (folder, files) in folders {
        if files > MAX_FILES_PER_FOLDER {
            offenders.push(format!("{}: {files} code files", folder.display()));
        }
    }
    offenders.sort();
    (checked, offenders)
}

fn collect_code_files(dir: &Path, unreadable: &mut Vec<String>, visit: &mut impl FnMut(&Path)) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
        Err(e) => {
            unreadable.push(format!("{}: unreadable folder ({e})", dir.display()));
            return;
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(e) => {
                unreadable.push(format!("{}: unreadable entry ({e})", dir.display()));
                continue;
            }
        };
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "target") {
                continue;
            }
            collect_code_files(&path, unreadable, visit);
        } else if path
            .extension()
            .is_some_and(|e| CODE_EXTENSIONS.iter().any(|code| e == *code))
        {
            visit(&path);
        }
    }
}

#[cfg(test)]
mod tests;
