use super::*;

pub(super) fn rust(src: &str, out: &mut Vec<Comment>) -> Result<(), String> {
    let b = src.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let next = b.get(i + 1).copied();
        match b[i] {
            b'/' if next == Some(b'/') => {
                let end = line_end(b, i);
                out.push(Comment { start: i, end });
                i = end;
            }
            b'/' if next == Some(b'*') => {
                let end =
                    rust_block_end(b, i).ok_or_else(|| unterminated(src, i, "block comment"))?;
                out.push(Comment { start: i, end });
                i = end;
            }
            b'"' => {
                i = quoted_end(b, i, b.len(), b'"', true)
                    .ok_or_else(|| unterminated(src, i, "string"))?;
            }
            b'\'' => i = rust_quote(src, i)?,
            c if c.is_ascii_digit() => i = word_end(b, i, false),
            c if is_word_byte(c) => i = rust_word(src, i)?,
            _ => i += 1,
        }
    }
    Ok(())
}

pub(super) fn rust_block_end(b: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut j = open;
    while j + 1 < b.len() {
        if b[j] == b'/' && b[j + 1] == b'*' {
            depth += 1;
            j += 2;
        } else if b[j] == b'*' && b[j + 1] == b'/' {
            depth -= 1;
            j += 2;
            if depth == 0 {
                return Some(j);
            }
        } else {
            j += 1;
        }
    }
    None
}

pub(super) fn rust_quote(src: &str, open: usize) -> Result<usize, String> {
    let b = src.as_bytes();
    let Some(first) = src[open + 1..].chars().next() else {
        return Ok(open + 1);
    };
    if first == '\\' {
        let escaped = open + 2;
        let skip = src[escaped..].chars().next().map_or(0, char::len_utf8);
        let mut j = escaped + skip;
        while j < b.len() && b[j] != b'\'' {
            if b[j] == b'\n' {
                return Err(unterminated(src, open, "character literal"));
            }
            j += 1;
        }
        if j == b.len() {
            return Err(unterminated(src, open, "character literal"));
        }
        return Ok(j + 1);
    }
    let after = open + 1 + first.len_utf8();
    if b.get(after) == Some(&b'\'') {
        Ok(after + 1)
    } else {
        Ok(open + 1)
    }
}

pub(super) fn rust_word(src: &str, start: usize) -> Result<usize, String> {
    let b = src.as_bytes();
    let end = word_end(b, start, false);
    match &src[start..end] {
        "r" | "br" | "cr" => {
            let mut k = end;
            while b.get(k) == Some(&b'#') {
                k += 1;
            }
            if b.get(k) != Some(&b'"') {
                return Ok(end);
            }
            let hashes = k - end;
            let mut j = k + 1;
            while j < b.len() {
                if b[j] == b'"'
                    && b.get(j + 1..j + 1 + hashes)
                        .is_some_and(|h| h.iter().all(|&c| c == b'#'))
                {
                    return Ok(j + 1 + hashes);
                }
                j += 1;
            }
            Err(unterminated(src, start, "raw string"))
        }
        "b" | "c" if b.get(end) == Some(&b'"') => quoted_end(b, end, b.len(), b'"', true)
            .ok_or_else(|| unterminated(src, start, "string")),
        "b" if b.get(end) == Some(&b'\'') => rust_quote(src, end),
        _ => Ok(end),
    }
}

pub(super) const REGEX_AFTER: [&str; 15] = [
    "return",
    "typeof",
    "instanceof",
    "in",
    "of",
    "new",
    "delete",
    "void",
    "throw",
    "case",
    "do",
    "else",
    "yield",
    "await",
    "extends",
];

pub(super) struct Script<'a> {
    pub(super) src: &'a str,
    pub(super) b: &'a [u8],
    pub(super) i: usize,
    pub(super) out: &'a mut Vec<Comment>,
}

impl<'a> Script<'a> {
    pub(super) fn at(src: &'a str, from: usize, out: &'a mut Vec<Comment>) -> Self {
        Self {
            src,
            b: src.as_bytes(),
            i: from,
            out,
        }
    }

    pub(super) fn code(&mut self, to: usize, inside_template: bool) -> Result<(), String> {
        let mut depth = 0usize;
        let mut regex_allowed = true;
        while self.i < to {
            let c = self.b[self.i];
            let next = if self.i + 1 < to {
                Some(self.b[self.i + 1])
            } else {
                None
            };
            match c {
                b'/' if next == Some(b'/') => {
                    let end = line_end(self.b, self.i).min(to);
                    self.out.push(Comment { start: self.i, end });
                    self.i = end;
                }
                b'/' if next == Some(b'*') => {
                    let close = find(self.b, self.i + 2, to, b"*/")
                        .ok_or_else(|| unterminated(self.src, self.i, "block comment"))?;
                    self.out.push(Comment {
                        start: self.i,
                        end: close + 2,
                    });
                    self.i = close + 2;
                }
                b'/' if regex_allowed => {
                    self.i = self.regex_end(to)?;
                    regex_allowed = false;
                }
                b'"' | b'\'' => {
                    self.i = quoted_end(self.b, self.i, to, c, true)
                        .ok_or_else(|| unterminated(self.src, self.i, "string"))?;
                    regex_allowed = false;
                }
                b'`' => {
                    self.template(to)?;
                    regex_allowed = false;
                }
                b'{' => {
                    depth += 1;
                    self.i += 1;
                    regex_allowed = true;
                }
                b'}' if inside_template && depth == 0 => {
                    self.i += 1;
                    return Ok(());
                }
                b'}' => {
                    depth = depth.saturating_sub(1);
                    self.i += 1;
                    regex_allowed = false;
                }
                b')' | b']' => {
                    self.i += 1;
                    regex_allowed = false;
                }
                b'+' | b'-' if next == Some(c) => {
                    self.i += 2;
                    regex_allowed = false;
                }
                c if is_word_byte(c) || c == b'$' => {
                    let end = word_end(self.b, self.i, true);
                    regex_allowed = REGEX_AFTER.contains(&&self.src[self.i..end]);
                    self.i = end;
                }
                c if c.is_ascii_whitespace() => self.i += 1,
                _ => {
                    self.i += 1;
                    regex_allowed = true;
                }
            }
        }
        if inside_template {
            return Err(unterminated(self.src, self.i, "template expression"));
        }
        Ok(())
    }

    pub(super) fn regex_end(&self, to: usize) -> Result<usize, String> {
        let mut j = self.i + 1;
        let mut in_class = false;
        while j < to {
            match self.b[j] {
                b'\\' => j += 1,
                b'[' => in_class = true,
                b']' => in_class = false,
                b'/' if !in_class => return Ok(word_end(self.b, j + 1, false)),
                b'\n' => break,
                _ => {}
            }
            j += 1;
        }
        Err(unterminated(self.src, self.i, "regular expression"))
    }

    pub(super) fn template(&mut self, to: usize) -> Result<(), String> {
        let open = self.i;
        self.i += 1;
        while self.i < to {
            match self.b[self.i] {
                b'\\' => self.i += 2,
                b'`' => {
                    self.i += 1;
                    return Ok(());
                }
                b'$' if self.b.get(self.i + 1) == Some(&b'{') => {
                    self.i += 2;
                    self.code(to, true)?;
                }
                _ => self.i += 1,
            }
        }
        Err(unterminated(self.src, open, "template literal"))
    }
}

pub(super) fn css(src: &str, from: usize, to: usize, out: &mut Vec<Comment>) -> Result<(), String> {
    let b = src.as_bytes();
    let mut i = from;
    while i < to {
        match b[i] {
            b'/' if i + 1 < to && b[i + 1] == b'*' => {
                let close =
                    find(b, i + 2, to, b"*/").ok_or_else(|| unterminated(src, i, "CSS comment"))?;
                out.push(Comment {
                    start: i,
                    end: close + 2,
                });
                i = close + 2;
            }
            q @ (b'"' | b'\'') => {
                i = quoted_end(b, i, to, q, true)
                    .ok_or_else(|| unterminated(src, i, "CSS string"))?;
            }
            _ => i += 1,
        }
    }
    Ok(())
}

pub(super) fn slint(src: &str, out: &mut Vec<Comment>) -> Result<(), String> {
    let b = src.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'/' if b.get(i + 1) == Some(&b'/') => {
                let end = line_end(b, i);
                out.push(Comment { start: i, end });
                i = end;
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                let close = find(b, i + 2, b.len(), b"*/")
                    .ok_or_else(|| unterminated(src, i, "Slint comment"))?;
                out.push(Comment {
                    start: i,
                    end: close + 2,
                });
                i = close + 2;
            }
            b'"' => {
                i = quoted_end(b, i, b.len(), b'"', true)
                    .ok_or_else(|| unterminated(src, i, "Slint string"))?;
            }
            _ => i += 1,
        }
    }
    Ok(())
}

pub(super) fn html(src: &str, out: &mut Vec<Comment>) -> Result<(), String> {
    let b = src.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i..].starts_with(b"<!--") {
            let close = find(b, i + 4, b.len(), b"-->")
                .ok_or_else(|| unterminated(src, i, "HTML comment"))?;
            out.push(Comment {
                start: i,
                end: close + 3,
            });
            i = close + 3;
            continue;
        }
        if b[i] != b'<' || !b.get(i + 1).is_some_and(u8::is_ascii_alphabetic) {
            i += 1;
            continue;
        }
        let name_end = word_end(b, i + 1, false);
        let name = src[i + 1..name_end].to_ascii_lowercase();
        let mut j = name_end;
        while j < b.len() && b[j] != b'>' {
            if b[j] == b'"' || b[j] == b'\'' {
                j = quoted_end(b, j, b.len(), b[j], false)
                    .ok_or_else(|| unterminated(src, j, "attribute value"))?;
            } else {
                j += 1;
            }
        }
        let content = (j + 1).min(b.len());
        let closing: &[u8] = match name.as_str() {
            "script" => b"</script",
            "style" => b"</style",
            _ => {
                i = content;
                continue;
            }
        };
        let content_end =
            find_ignore_case(b, content, closing).ok_or_else(|| unterminated(src, i, &name))?;
        if name == "script" {
            Script::at(src, content, out).code(content_end, false)?;
        } else {
            css(src, content, content_end, out)?;
        }
        i = content_end;
    }
    Ok(())
}
