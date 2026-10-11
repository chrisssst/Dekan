use super::*;

pub(super) fn toml(src: &str, out: &mut Vec<Comment>) -> Result<(), String> {
    let b = src.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'#' => {
                let end = line_end(b, i);
                out.push(Comment { start: i, end });
                i = end;
            }
            b'"' if b[i..].starts_with(b"\"\"\"") => {
                let mut j = i + 3;
                loop {
                    if j + 3 > b.len() {
                        return Err(unterminated(src, i, "multi-line string"));
                    }
                    if b[j] == b'\\' {
                        j += 2;
                    } else if b[j..].starts_with(b"\"\"\"") {
                        break;
                    } else {
                        j += 1;
                    }
                }
                i = j + 3;
            }
            b'\'' if b[i..].starts_with(b"'''") => {
                let close = find(b, i + 3, b.len(), b"'''")
                    .ok_or_else(|| unterminated(src, i, "multi-line literal string"))?;
                i = close + 3;
            }
            q @ (b'"' | b'\'') => {
                i = quoted_end(b, i, b.len(), q, q == b'"')
                    .ok_or_else(|| unterminated(src, i, "string"))?;
            }
            _ => i += 1,
        }
    }
    Ok(())
}

pub(super) struct YamlLine {
    pub(super) key: Option<String>,
    pub(super) key_column: usize,
    pub(super) block_scalar: bool,
    pub(super) value: String,
}

pub(super) fn yaml(src: &str, out: &mut Vec<Comment>) -> Result<(), String> {
    let spans = lines(src);
    let mut open_quote: Option<u8> = None;
    let mut windows_runner = false;
    let mut idx = 0;
    while idx < spans.len() {
        let (start, end) = spans[idx];
        let line = yaml_line(src, start, end, &mut open_quote, out);
        if line.key.as_deref() == Some("runs-on") {
            windows_runner = line.value.contains("windows");
        }
        if !line.block_scalar {
            idx += 1;
            continue;
        }
        let parent = indent(&src[start..end]);
        let mut content_indent = None;
        let mut k = idx + 1;
        while k < spans.len() {
            let text = &src[spans[k].0..spans[k].1];
            if text.trim().is_empty() {
                k += 1;
                continue;
            }
            let this = indent(text);
            match content_indent {
                None if this > parent => content_indent = Some(this),
                None => break,
                Some(ci) if this < ci => break,
                Some(_) => {}
            }
            k += 1;
        }
        if line.key.as_deref() == Some("run") && k > idx + 1 {
            let shell =
                step_shell(src, &spans, idx, k, line.key_column).unwrap_or(if windows_runner {
                    Shell::Pwsh
                } else {
                    Shell::Bash
                });
            shell_comments(src, &spans[idx + 1..k], shell, out)?;
        }
        idx = k;
    }
    if open_quote.is_some() {
        return Err("unterminated quoted scalar at end of file".into());
    }
    Ok(())
}

pub(super) fn yaml_line(
    src: &str,
    start: usize,
    end: usize,
    open_quote: &mut Option<u8>,
    out: &mut Vec<Comment>,
) -> YamlLine {
    let b = src.as_bytes();
    let mut i = start;
    let mut result = YamlLine {
        key: None,
        key_column: 0,
        block_scalar: false,
        value: String::new(),
    };
    if let Some(q) = *open_quote {
        match yaml_quote_end(b, i, end, q) {
            Some(j) => {
                *open_quote = None;
                i = j;
            }
            None => return result,
        }
    }
    let mut value_start_ok = true;
    let mut content_start: Option<usize> = None;
    let mut value_from: Option<usize> = None;
    let mut code_end = end;
    let mut flow = 0usize;
    while i < end {
        let c = b[i];
        match c {
            b' ' | b'\t' => i += 1,
            b'#' if i == start || b[i - 1] == b' ' || b[i - 1] == b'\t' => {
                out.push(Comment { start: i, end });
                code_end = i;
                break;
            }
            b'\'' | b'"' if value_start_ok => match yaml_quote_end(b, i + 1, end, c) {
                Some(j) => {
                    content_start.get_or_insert(i);
                    i = j;
                    value_start_ok = false;
                }
                None => {
                    *open_quote = Some(c);
                    return result;
                }
            },
            b'-' if value_start_ok
                && content_start.is_none()
                && (i + 1 == end || b[i + 1] == b' ') =>
            {
                i += 1;
            }
            b':' if flow == 0
                && result.key.is_none()
                && (i + 1 == end || b[i + 1] == b' ' || b[i + 1] == b'\t') =>
            {
                if let Some(key_start) = content_start {
                    result.key = Some(src[key_start..i].trim().to_string());
                    result.key_column = key_start - start;
                }
                i += 1;
                value_start_ok = true;
                value_from = Some(i);
            }
            b'[' | b'{' if value_start_ok => {
                flow += 1;
                content_start.get_or_insert(i);
                i += 1;
            }
            b']' | b'}' if flow > 0 => {
                flow -= 1;
                i += 1;
                value_start_ok = false;
            }
            b',' if flow > 0 => {
                i += 1;
                value_start_ok = true;
            }
            _ => {
                content_start.get_or_insert(i);
                value_start_ok = false;
                i += 1;
            }
        }
    }
    if let Some(from) = value_from {
        let value = src[from..code_end.max(from)].trim();
        result.value = value.to_string();
        let mut chars = value.chars();
        result.block_scalar = matches!(chars.next(), Some('|' | '>'))
            && chars.all(|ch| ch == '+' || ch == '-' || ch.is_ascii_digit());
    }
    result
}

pub(super) fn yaml_quote_end(b: &[u8], from: usize, end: usize, quote: u8) -> Option<usize> {
    let mut j = from;
    while j < end {
        if quote == b'"' && b[j] == b'\\' {
            j += 2;
            continue;
        }
        if b[j] == quote {
            if quote == b'\'' && b.get(j + 1) == Some(&b'\'') {
                j += 2;
                continue;
            }
            return Some(j + 1);
        }
        j += 1;
    }
    None
}

pub(super) fn inno(src: &str, out: &mut Vec<Comment>) -> Result<(), String> {
    for (start, end) in lines(src) {
        let text = &src[start..end];
        let trimmed = text.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            if trimmed.eq_ignore_ascii_case("[code]") {
                return pascal(src, end, out);
            }
            continue;
        }
        if trimmed.starts_with(';') {
            out.push(Comment {
                start: start + indent(text),
                end,
            });
        }
    }
    Ok(())
}

pub(super) fn pascal(src: &str, from: usize, out: &mut Vec<Comment>) -> Result<(), String> {
    let b = src.as_bytes();
    let mut i = from;
    while i < b.len() {
        match b[i] {
            b'/' if b.get(i + 1) == Some(&b'/') => {
                let end = line_end(b, i);
                out.push(Comment { start: i, end });
                i = end;
            }
            b'{' if b.get(i + 1) == Some(&b'#') => {
                i = find(b, i, b.len(), b"}")
                    .ok_or_else(|| unterminated(src, i, "preprocessor expansion"))?
                    + 1;
            }
            b'{' => {
                let close =
                    find(b, i, b.len(), b"}").ok_or_else(|| unterminated(src, i, "comment"))?;
                out.push(Comment {
                    start: i,
                    end: close + 1,
                });
                i = close + 1;
            }
            b'(' if b.get(i + 1) == Some(&b'*') => {
                let close = find(b, i + 2, b.len(), b"*)")
                    .ok_or_else(|| unterminated(src, i, "comment"))?;
                out.push(Comment {
                    start: i,
                    end: close + 2,
                });
                i = close + 2;
            }
            b'\'' => {
                i = quoted_end(b, i, b.len(), b'\'', false)
                    .ok_or_else(|| unterminated(src, i, "string"))?;
            }
            _ => i += 1,
        }
    }
    Ok(())
}
