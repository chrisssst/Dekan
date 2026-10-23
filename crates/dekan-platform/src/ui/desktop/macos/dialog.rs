use std::path::PathBuf;

use super::{run_script, was_canceled};
use crate::error::PlatformError;

pub fn pick_file(
    _owner: isize,
    title: &str,
    _filter_label: &str,
    patterns: &str,
) -> Result<Option<PathBuf>, PlatformError> {
    let types = extensions(patterns)
        .iter()
        .map(|ext| format!("\"{ext}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let choose = if types.is_empty() {
        "POSIX path of (choose file with prompt (item 1 of argv))".to_owned()
    } else {
        format!("POSIX path of (choose file with prompt (item 1 of argv) of type {{{types}}})")
    };
    let output = run_script(&[&choose], &[title]).map_err(|source| PlatformError::Io {
        context: "starting the file dialog".into(),
        source,
    })?;
    if was_canceled(&output) {
        return Ok(None);
    }
    if !output.status.success() {
        return Err(PlatformError::Window(format!(
            "file dialog failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let chosen = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    Ok((!chosen.is_empty()).then(|| PathBuf::from(chosen)))
}

fn extensions(patterns: &str) -> Vec<String> {
    patterns
        .split(';')
        .filter_map(|pattern| pattern.trim().strip_prefix("*."))
        .filter(|ext| !ext.is_empty() && ext.chars().all(|c| c.is_ascii_alphanumeric()))
        .map(str::to_ascii_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_style_patterns_become_plain_extensions() {
        assert_eq!(
            extensions("*.fantome;*.ZIP; *.modpkg"),
            ["fantome", "zip", "modpkg"]
        );
        assert!(extensions("*.*;\"x").is_empty());
    }
}
