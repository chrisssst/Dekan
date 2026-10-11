#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Rust,
    Script,
    Css,
    Html,
    Slint,
    Toml,
    Yaml,
    Inno,
}

impl Language {
    pub fn of(path: &str) -> Option<Self> {
        let extension = path.rsplit_once('.')?.1.to_ascii_lowercase();
        match extension.as_str() {
            "rs" => Some(Self::Rust),
            "ts" | "js" | "mjs" | "cjs" => Some(Self::Script),
            "css" => Some(Self::Css),
            "html" | "htm" => Some(Self::Html),
            "slint" => Some(Self::Slint),
            "toml" => Some(Self::Toml),
            "yml" | "yaml" => Some(Self::Yaml),
            "iss" => Some(Self::Inno),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Comment {
    pub start: usize,
    pub end: usize,
}

pub fn lex(language: Language, src: &str) -> Result<Vec<Comment>, String> {
    let mut out = Vec::new();
    match language {
        Language::Rust => rust(src, &mut out)?,
        Language::Script => Script::at(src, 0, &mut out).code(src.len(), false)?,
        Language::Css => css(src, 0, src.len(), &mut out)?,
        Language::Html => html(src, &mut out)?,
        Language::Slint => slint(src, &mut out)?,
        Language::Toml => toml(src, &mut out)?,
        Language::Yaml => yaml(src, &mut out)?,
        Language::Inno => inno(src, &mut out)?,
    }
    out.sort_by_key(|c| c.start);
    Ok(out)
}

pub fn is_directive(language: Language, src: &str, comment: Comment) -> bool {
    let text = src[comment.start..comment.end].trim();
    if text.contains('\n') {
        return false;
    }
    match language {
        Language::Rust => text.starts_with("// ignore-ok:"),
        Language::Script => {
            text.starts_with("// @ts-")
                || text.starts_with("/// <reference")
                || text.starts_with("// eslint-")
        }
        Language::Yaml => {
            text.starts_with("# zizmor:")
                || text.starts_with("# yaml-language-server:")
                || text.starts_with("# shellcheck ")
                || follows_pinned_action(src, comment)
        }
        Language::Toml => text.starts_with("#:schema"),
        Language::Css | Language::Html | Language::Slint | Language::Inno => false,
    }
}

pub fn line_of(src: &str, pos: usize) -> usize {
    src.as_bytes()[..pos.min(src.len())]
        .iter()
        .filter(|&&b| b == b'\n')
        .count()
        + 1
}

fn follows_pinned_action(src: &str, comment: Comment) -> bool {
    let line_start = src[..comment.start].rfind('\n').map_or(0, |p| p + 1);
    let code = src[line_start..comment.start].trim();
    let code = code.strip_prefix("- ").map_or(code, str::trim_start);
    code.starts_with("uses:") && code.contains('@')
}

fn unterminated(src: &str, pos: usize, what: &str) -> String {
    format!("line {}: unterminated {what}", line_of(src, pos))
}

fn line_end(b: &[u8], from: usize) -> usize {
    let mut j = from;
    while j < b.len() && b[j] != b'\n' {
        j += 1;
    }
    if j > from && b[j - 1] == b'\r' {
        j - 1
    } else {
        j
    }
}

fn find(b: &[u8], from: usize, to: usize, needle: &[u8]) -> Option<usize> {
    let last = to.checked_sub(needle.len())?;
    (from..=last).find(|&j| &b[j..j + needle.len()] == needle)
}

fn find_ignore_case(b: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    let last = b.len().checked_sub(needle.len())?;
    (from..=last).find(|&j| b[j..j + needle.len()].eq_ignore_ascii_case(needle))
}

fn quoted_end(b: &[u8], open: usize, to: usize, quote: u8, escapes: bool) -> Option<usize> {
    let mut j = open + 1;
    while j < to {
        if escapes && b[j] == b'\\' {
            j += 2;
            continue;
        }
        if b[j] == quote {
            return Some(j + 1);
        }
        j += 1;
    }
    None
}

fn is_word_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80
}

fn word_end(b: &[u8], from: usize, dollar: bool) -> usize {
    let mut j = from;
    while j < b.len() && (is_word_byte(b[j]) || (dollar && b[j] == b'$')) {
        j += 1;
    }
    j
}

fn lines(src: &str) -> Vec<(usize, usize)> {
    let b = src.as_bytes();
    let mut spans = Vec::new();
    let mut start = 0;
    while start < b.len() {
        let end = line_end(b, start);
        spans.push((start, end));
        let mut next = end;
        while next < b.len() && b[next] != b'\n' {
            next += 1;
        }
        start = next + 1;
    }
    spans
}

fn indent(line: &str) -> usize {
    line.len() - line.trim_start_matches([' ', '\t']).len()
}

mod code;
mod config;
mod shell;

use code::{Script, css, html, rust, slint};
use config::{inno, toml, yaml};
use shell::{Shell, shell_comments, step_shell};
