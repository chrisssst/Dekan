use std::path::Path;

pub(crate) fn run_set_version(args: &[String]) {
    let Some(display) = args.first() else {
        eprintln!("usage: cargo xtask set-version <major.minor>");
        std::process::exit(2);
    };
    let Some(version) = cargo_version(display) else {
        eprintln!("[ERROR] {display:?} is not a major.minor version");
        std::process::exit(2);
    };
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let stamp = |file: &str, rewrite: fn(&str, &str) -> Result<String, String>| {
        let path = root.join(file);
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            eprintln!("[ERROR] {}: {e}", path.display());
            std::process::exit(1);
        });
        match rewrite(&text, &version) {
            Ok(next) => {
                if let Err(e) = std::fs::write(&path, next) {
                    eprintln!("[ERROR] {}: {e}", path.display());
                    std::process::exit(1);
                }
            }
            Err(e) => {
                eprintln!("[ERROR] {file}: {e}");
                std::process::exit(1);
            }
        }
    };
    stamp("Cargo.toml", stamp_manifest);
    stamp("Cargo.lock", stamp_lock);
    println!("[OK] Workspace version set to {version} (displayed as {display})");
}

pub(crate) fn cargo_version(display: &str) -> Option<String> {
    let mut parts = display.split('.');
    let major: u32 = parts.next()?.parse().ok()?;
    let minor: u32 = parts.next()?.parse().ok()?;
    parts.next().is_none().then(|| format!("{major}.{minor}.0"))
}

pub(crate) fn stamp_manifest(text: &str, version: &str) -> Result<String, String> {
    let mut in_package = false;
    let mut stamped = false;
    let next: String = text
        .split_inclusive('\n')
        .map(|line| {
            let trimmed = line.trim();
            if trimmed.starts_with('[') {
                in_package = trimmed == "[workspace.package]";
            } else if in_package
                && !stamped
                && trimmed.starts_with("version")
                && trimmed.contains('=')
            {
                stamped = true;
                return format!("version = \"{version}\"{}", &line[line.trim_end().len()..]);
            }
            line.to_owned()
        })
        .collect();
    stamped
        .then_some(next)
        .ok_or_else(|| "no version under [workspace.package]".to_owned())
}

pub(crate) fn stamp_lock(text: &str, version: &str) -> Result<String, String> {
    let mut stamped = 0;
    let mut in_member = false;
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let mut next = String::with_capacity(text.len());
    for (index, line) in lines.iter().enumerate() {
        if line.trim_end() == "[[package]]" {
            in_member = lines[index + 1..]
                .iter()
                .take_while(|later| later.trim_end() != "[[package]]")
                .all(|later| !later.starts_with("source = "));
        }
        match line.strip_prefix("version = ") {
            Some(rest) if in_member => {
                stamped += 1;
                next.push_str(&format!(
                    "version = \"{version}\"{}",
                    &rest[rest.trim_end().len()..]
                ));
            }
            _ => next.push_str(line),
        }
    }
    if stamped == 0 {
        return Err("no workspace member in the lock file".to_owned());
    }
    Ok(next)
}

#[cfg(test)]
mod tests;
