use std::path::{Path, PathBuf};
use std::process::Command;

use crate::comment_lexers::{Comment, Language, is_directive, lex, line_of};

pub struct Finding {
    pub path: String,
    pub line: usize,
    pub text: String,
}

pub fn run(args: &[String]) {
    let strip = args.iter().any(|a| a == "--strip");
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf();
    let report = args
        .iter()
        .position(|a| a == "--report")
        .and_then(|p| args.get(p + 1))
        .map_or_else(|| root.join("target/comments-removed.md"), PathBuf::from);

    let files = match tracked_files(&root) {
        Ok(files) => files,
        Err(e) => {
            eprintln!("[ERROR] comments: {e}");
            std::process::exit(1);
        }
    };

    let mut findings = Vec::new();
    let mut failures = Vec::new();
    let mut directives = 0usize;
    let mut changed = 0usize;
    for (path, language) in &files {
        let full = root.join(path);
        let Ok(src) = std::fs::read_to_string(&full) else {
            failures.push(format!("{path}: not readable as UTF-8"));
            continue;
        };
        let comments = match lex(*language, &src) {
            Ok(comments) => comments,
            Err(e) => {
                failures.push(format!("{path}: {e}"));
                continue;
            }
        };
        let (kept, removable): (Vec<Comment>, Vec<Comment>) = comments
            .into_iter()
            .partition(|c| is_directive(*language, &src, *c));
        directives += kept.len();
        if removable.is_empty() {
            continue;
        }
        for c in &removable {
            findings.push(Finding {
                path: path.clone(),
                line: line_of(&src, c.start),
                text: src[c.start..c.end].to_string(),
            });
        }
        if !strip {
            continue;
        }
        let stripped = strip_comments(&src, &removable);
        match verify(*language, &src, &stripped) {
            Ok(()) => {
                if let Err(e) = std::fs::write(&full, stripped) {
                    failures.push(format!("{path}: {e}"));
                } else {
                    changed += 1;
                }
            }
            Err(e) => failures.push(format!("{path}: left untouched, {e}")),
        }
    }

    println!(
        "  {} files, {} comments, {} tool directives kept",
        files.len(),
        findings.len(),
        directives
    );
    if strip {
        if let Err(e) = write_report(&report, &findings) {
            failures.push(format!("{}: {e}", report.display()));
        }
        println!(
            "  {changed} files cleaned; removed text in {}",
            report.display()
        );
    } else if !findings.is_empty() {
        eprintln!(
            "\n[ERROR] Comments in code ({}); explanations belong in docs/, crate READMEs or ADRs:",
            findings.len()
        );
        for f in &findings {
            let first = f.text.lines().next().unwrap_or_default().trim();
            eprintln!("  {}:{}: {first}", f.path, f.line);
        }
        eprintln!("  `cargo xtask comments --strip` removes them and lists the removed text.");
        std::process::exit(1);
    }
    if !failures.is_empty() {
        eprintln!("\n[ERROR] comments:");
        for failure in &failures {
            eprintln!("  {failure}");
        }
        std::process::exit(1);
    }
}

fn tracked_files(root: &Path) -> Result<Vec<(String, Language)>, String> {
    let output = Command::new("git")
        .args(["ls-files", "-z"])
        .current_dir(root)
        .output()
        .map_err(|e| format!("git ls-files: {e}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    let listing = String::from_utf8(output.stdout).map_err(|e| format!("git ls-files: {e}"))?;
    Ok(listing
        .split('\0')
        .filter(|p| !p.is_empty())
        .filter_map(|p| Language::of(p).map(|l| (p.to_string(), l)))
        .filter(|(p, _)| root.join(p).is_file())
        .collect())
}

pub fn strip_comments(src: &str, removable: &[Comment]) -> String {
    let mut text = src.to_string();
    for c in removable.iter().rev() {
        let line_start = text[..c.start].rfind('\n').map_or(0, |p| p + 1);
        let line_end = text[c.end..].find('\n').map_or(text.len(), |p| c.end + p);
        let before = &text[line_start..c.start];
        let after = &text[c.end..line_end];
        if before.trim().is_empty() && after.trim().is_empty() {
            let (from, to) = if line_end < text.len() {
                (line_start, line_end + 1)
            } else {
                (line_start.saturating_sub(1), line_end)
            };
            text.replace_range(from..to, "");
            drop_doubled_blank_line(&mut text, from);
        } else {
            let (range, replacement) = inline_removal(&text, c.start, c.end, line_start, line_end);
            text.replace_range(range, replacement);
        }
    }
    text
}

fn inline_removal(
    text: &str,
    start: usize,
    end: usize,
    line_start: usize,
    line_end: usize,
) -> (std::ops::Range<usize>, &'static str) {
    let before = &text[line_start..start];
    let after = &text[end..line_end];
    if after.trim().is_empty() {
        let from = line_start + before.trim_end().len();
        let keep = if after.ends_with('\r') { "\r" } else { "" };
        return (from..line_end, keep);
    }
    if before.trim().is_empty() {
        let spaces = after.len() - after.trim_start_matches([' ', '\t']).len();
        return (start..end + spaces, "");
    }
    let left = text[..start].chars().next_back();
    let right = text[end..].chars().next();
    let joins =
        left.is_some_and(|l| !l.is_whitespace()) && right.is_some_and(|r| !r.is_whitespace());
    (start..end, if joins { " " } else { "" })
}

fn drop_doubled_blank_line(text: &mut String, at: usize) {
    let next_end = text[at..].find('\n').map(|p| at + p + 1);
    let Some(next_end) = next_end else {
        if text[at..].trim().is_empty() && text[..at].ends_with("\n\n") {
            text.truncate(at - 1);
        }
        return;
    };
    if !text[at..next_end].trim().is_empty() {
        return;
    }
    let previous_blank = at == 0 || {
        let previous_start = text[..at - 1].rfind('\n').map_or(0, |p| p + 1);
        text[previous_start..at].trim().is_empty()
    };
    let next_is_end = text[next_end..].trim().is_empty();
    if previous_blank || next_is_end {
        text.replace_range(at..next_end, "");
    }
}

pub fn verify(language: Language, original: &str, stripped: &str) -> Result<(), String> {
    let before = lex(language, original)?;
    let after = lex(language, stripped).map_err(|e| format!("the result no longer lexes: {e}"))?;
    let left_over: Vec<usize> = after
        .iter()
        .filter(|c| !is_directive(language, stripped, **c))
        .map(|c| line_of(stripped, c.start))
        .collect();
    if !left_over.is_empty() {
        return Err(format!("comments remain at lines {left_over:?}"));
    }
    let directives = |src: &str, comments: &[Comment]| -> Vec<String> {
        comments
            .iter()
            .filter(|c| is_directive(language, src, **c))
            .map(|c| src[c.start..c.end].to_string())
            .collect()
    };
    if directives(original, &before) != directives(stripped, &after) {
        return Err("tool directives changed".into());
    }
    let code_before = code_lines(original, &before);
    let code_after = code_lines(stripped, &after);
    if let Some(n) = (0..code_before.len().max(code_after.len()))
        .find(|&n| code_before.get(n) != code_after.get(n))
    {
        return Err(format!(
            "code changed: {:?} became {:?}",
            code_before.get(n),
            code_after.get(n)
        ));
    }
    Ok(())
}

fn code_lines(src: &str, comments: &[Comment]) -> Vec<String> {
    let mut text = src.to_string();
    for c in comments.iter().rev() {
        let line_start = text[..c.start].rfind('\n').map_or(0, |p| p + 1);
        let line_end = text[c.end..].find('\n').map_or(text.len(), |p| c.end + p);
        let (range, replacement) = inline_removal(&text, c.start, c.end, line_start, line_end);
        text.replace_range(range, replacement);
    }
    text.lines()
        .map(str::trim_end)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

fn write_report(path: &Path, findings: &[Finding]) -> std::io::Result<()> {
    let mut out = String::from("# Comments removed from code\n\n");
    let mut current = "";
    for f in findings {
        if f.path != current {
            out.push_str(&format!("\n## {}\n\n", f.path));
            current = &f.path;
        }
        out.push_str(&format!("- L{}: ", f.line));
        let lines: Vec<&str> = f.text.lines().map(str::trim).collect();
        out.push_str(&lines.join(" "));
        out.push('\n');
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, out)
}

#[cfg(test)]
mod tests;
