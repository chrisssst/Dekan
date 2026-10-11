use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Shell {
    Bash,
    Pwsh,
}

pub(super) fn key_line(text: &str) -> Option<(usize, &str, &str)> {
    let ind = indent(text);
    let rest = &text[ind..];
    let (column, rest) = match rest.strip_prefix("- ") {
        Some(item) => (ind + 2, item),
        None => (ind, rest),
    };
    let (key, value) = rest.split_once(':')?;
    Some((column, key.trim(), value.trim()))
}

pub(super) fn step_shell(
    src: &str,
    spans: &[(usize, usize)],
    run_line: usize,
    after_block: usize,
    column: usize,
) -> Option<Shell> {
    let as_shell = |value: &str| {
        let value = value.trim_matches(|c| c == '\'' || c == '"');
        if value.starts_with("pwsh") || value.starts_with("powershell") {
            Shell::Pwsh
        } else {
            Shell::Bash
        }
    };
    let opens_item = src[spans[run_line].0..spans[run_line].1]
        .trim_start()
        .starts_with("- ");
    let before = if opens_item {
        &[][..]
    } else {
        &spans[..run_line]
    };
    for &(s, e) in before.iter().rev() {
        let text = &src[s..e];
        if text.trim().is_empty() {
            continue;
        }
        let ind = indent(text);
        if let Some(value) = shell_value(text, column) {
            return Some(as_shell(value));
        }
        if ind < column {
            break;
        }
    }
    for &(s, e) in &spans[after_block..] {
        let text = &src[s..e];
        if text.trim().is_empty() {
            continue;
        }
        if indent(text) < column {
            break;
        }
        if let Some(value) = shell_value(text, column) {
            return Some(as_shell(value));
        }
    }
    None
}

pub(super) fn shell_value(text: &str, column: usize) -> Option<&str> {
    let (col, key, value) = key_line(text)?;
    (col == column && key == "shell").then_some(value)
}

pub(super) fn shell_comments(
    src: &str,
    spans: &[(usize, usize)],
    shell: Shell,
    out: &mut Vec<Comment>,
) -> Result<(), String> {
    let b = src.as_bytes();
    let mut heredoc: Option<String> = None;
    let mut here_string: Option<u8> = None;
    let mut block_comment: Option<usize> = None;
    let mut quote: Option<u8> = None;
    for &(start, end) in spans {
        let text = &src[start..end];
        if let Some(terminator) = &heredoc {
            if text.trim() == terminator {
                heredoc = None;
            }
            continue;
        }
        if let Some(q) = here_string {
            let t = text.trim_start().as_bytes();
            if t.len() >= 2 && t[0] == q && t[1] == b'@' {
                here_string = None;
            }
            continue;
        }
        let mut i = start;
        if let Some(open) = block_comment {
            match find(b, i, end, b"#>") {
                Some(close) => {
                    out.push(Comment {
                        start: open,
                        end: close + 2,
                    });
                    block_comment = None;
                    i = close + 2;
                }
                None => continue,
            }
        }
        while i < end {
            let c = b[i];
            if let Some(q) = quote {
                let escape = if shell == Shell::Bash { b'\\' } else { b'`' };
                if c == escape && q == b'"' {
                    i += 2;
                    continue;
                }
                if c == q {
                    quote = None;
                }
                i += 1;
                continue;
            }
            let word_start = src[start..i].trim().is_empty()
                || matches!(
                    (shell, b[i - 1]),
                    (_, b' ' | b'\t' | b';' | b'|' | b'(')
                        | (Shell::Bash, b'&')
                        | (Shell::Pwsh, b'{' | b'}')
                );
            match c {
                b'#' if word_start => {
                    out.push(Comment { start: i, end });
                    break;
                }
                b'<' if shell == Shell::Pwsh && word_start && b.get(i + 1) == Some(&b'#') => {
                    match find(b, i + 2, end, b"#>") {
                        Some(close) => {
                            out.push(Comment {
                                start: i,
                                end: close + 2,
                            });
                            i = close + 2;
                        }
                        None => {
                            block_comment = Some(i);
                            break;
                        }
                    }
                }
                b'\\' if shell == Shell::Bash => i += 2,
                b'`' if shell == Shell::Pwsh => i += 2,
                b'@' if shell == Shell::Pwsh
                    && matches!(b.get(i + 1), Some(b'\'' | b'"'))
                    && src[i + 2..end].trim().is_empty() =>
                {
                    here_string = Some(b[i + 1]);
                    break;
                }
                b'<' if shell == Shell::Bash
                    && b.get(i + 1) == Some(&b'<')
                    && b.get(i + 2) != Some(&b'<')
                    && (i == start || b[i - 1] != b'<') =>
                {
                    let mut j = i + 2;
                    if b.get(j) == Some(&b'-') {
                        j += 1;
                    }
                    while j < end && b[j] == b' ' {
                        j += 1;
                    }
                    let quoted = j < end && (b[j] == b'\'' || b[j] == b'"');
                    let word_from = if quoted { j + 1 } else { j };
                    let word_to = word_end(b, word_from, false);
                    heredoc = Some(src[word_from..word_to].to_string());
                    i = if quoted { word_to + 1 } else { word_to };
                }
                b'\'' | b'"' => {
                    quote = Some(c);
                    i += 1;
                }
                _ => i += 1,
            }
        }
    }
    if heredoc.is_some() || here_string.is_some() || block_comment.is_some() {
        return Err(unterminated(
            src,
            spans.first().map_or(0, |s| s.0),
            "shell block",
        ));
    }
    Ok(())
}
