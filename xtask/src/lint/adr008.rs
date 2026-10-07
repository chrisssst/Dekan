use std::path::PathBuf;

pub(crate) fn run_adr008_check() {
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf();

    let mut offenders = Vec::new();
    let mut checked = 0usize;
    let mut justified = 0usize;

    for dir in ["crates", "xtask"] {
        collect_rust_files(&workspace_root.join(dir), &mut |path| {
            let Ok(content) = std::fs::read_to_string(path) else {
                return;
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

    if offenders.is_empty() {
        println!("  {checked} discarded Results, {justified} justified, 0 unexplained");
        return;
    }

    eprintln!(
        "
[ERROR] Error-handling sweep: {} discarded Result(s) without a `// ignore-ok: <reason>`:",
        offenders.len()
    );
    for offender in &offenders {
        eprintln!("  {offender}");
    }
    std::process::exit(1);
}

pub(crate) fn collect_rust_files(dir: &std::path::Path, visit: &mut impl FnMut(&std::path::Path)) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "target") {
                continue;
            }
            collect_rust_files(&path, visit);
        } else if path.extension().is_some_and(|e| e == "rs") {
            visit(&path);
        }
    }
}
