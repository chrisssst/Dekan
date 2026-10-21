use std::path::PathBuf;

pub(crate) fn run_adr008_check() {
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf();

    let mut offenders = Vec::new();
    let mut unreadable = Vec::new();
    let mut checked = 0usize;
    let mut justified = 0usize;

    for dir in ["crates", "xtask"] {
        collect_rust_files(&workspace_root.join(dir), &mut unreadable, &mut |path| {
            let content = match std::fs::read_to_string(path) {
                Ok(content) => content,
                Err(e) => {
                    offenders.push(format!("{}: unreadable ({e})", path.display()));
                    return;
                }
            };
            let lines: Vec<&str> = content.lines().collect();
            for (i, line) in lines.iter().enumerate() {
                if !line.trim_start().starts_with("let _ = ") {
                    continue;
                }
                checked += 1;
                let previous = i.checked_sub(1).map(|p| lines[p]).unwrap_or("");
                if line.contains("ignore-ok") || previous.trim_start().starts_with("// ignore-ok") {
                    justified += 1;
                    continue;
                }
                offenders.push(format!("{}:{}: {}", path.display(), i + 1, line.trim()));
            }
        });
    }

    offenders.append(&mut unreadable);
    if offenders.is_empty() {
        println!("  {checked} discarded Results, {justified} justified, 0 unexplained");
        return;
    }

    eprintln!(
        "
[ERROR] Error-handling sweep: {} discarded Result(s) without a `// ignore-ok: <reason>`, or unreadable path(s):",
        offenders.len()
    );
    for offender in &offenders {
        eprintln!("  {offender}");
    }
    std::process::exit(1);
}

pub(crate) fn collect_rust_files(
    dir: &std::path::Path,
    unreadable: &mut Vec<String>,
    visit: &mut impl FnMut(&std::path::Path),
) {
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
            collect_rust_files(&path, unreadable, visit);
        } else if path.extension().is_some_and(|e| e == "rs") {
            visit(&path);
        }
    }
}
