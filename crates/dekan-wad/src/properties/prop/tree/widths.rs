use super::super::{
    Cursor, FIELD_EMBED, FIELD_HASH, FIELD_LIST, FIELD_LIST2, FIELD_MAP, FIELD_OPTION,
    FIELD_POINTER, FIELD_STRING, MAX_FIELD_DEPTH, fixed_field_size,
};
use super::read_fields;
use crate::error::WadError;

const NARROW: usize = 4;
const WIDE: usize = 8;
const STEPS_PER_BYTE: usize = 8;
const MIN_STEPS: usize = 4096;

pub fn hash_widths(body: &[u8]) -> Result<Vec<usize>, WadError> {
    if Search::new(body, false).body() {
        return Ok(Vec::new());
    }
    let mut search = Search::new(body, true);
    if search.body() {
        let mut wide = search.wide;
        wide.sort_unstable();
        return Ok(wide);
    }
    let mut cursor = Cursor::new(body, &[]);
    read_fields(&mut cursor, 0)?;
    if cursor.at != body.len() {
        return Err(WadError::InvalidProp(format!(
            "{} bytes after the last field",
            body.len() - cursor.at
        )));
    }
    Ok(Vec::new())
}

struct Search<'a> {
    data: &'a [u8],
    allow_wide: bool,
    wide: Vec<usize>,
    steps: usize,
}

struct Choice {
    index: u16,
    at: usize,
    kind: u8,
    mark: usize,
}

impl<'a> Search<'a> {
    fn new(data: &'a [u8], allow_wide: bool) -> Self {
        Self {
            data,
            allow_wide,
            wide: Vec::new(),
            steps: data
                .len()
                .saturating_mul(STEPS_PER_BYTE)
                .saturating_add(MIN_STEPS),
        }
    }

    fn body(&mut self) -> bool {
        self.fields(0, self.data.len(), 0)
    }

    fn step(&mut self) -> Option<()> {
        self.steps = self.steps.checked_sub(1)?;
        Some(())
    }

    fn byte(&self, at: usize, end: usize) -> Option<u8> {
        (at < end).then(|| self.data.get(at).copied()).flatten()
    }

    fn u16(&self, at: usize, end: usize) -> Option<usize> {
        let b = self.data.get(at..at.checked_add(2)?.min(end))?;
        Some(usize::from(u16::from_le_bytes(b.try_into().ok()?)))
    }

    fn u32(&self, at: usize, end: usize) -> Option<usize> {
        let b = self.data.get(at..at.checked_add(4)?.min(end))?;
        usize::try_from(u32::from_le_bytes(b.try_into().ok()?)).ok()
    }

    fn region(&self, start: usize, size: usize, end: usize) -> Option<usize> {
        start
            .checked_add(size)
            .filter(|region_end| *region_end <= end)
    }

    fn may_widen(&self, at: usize, kind: u8, end: usize) -> bool {
        self.allow_wide
            && match kind {
                FIELD_HASH => true,
                FIELD_OPTION => {
                    self.byte(at, end) == Some(FIELD_HASH)
                        && self.byte(at + 1, end).is_some_and(|present| present != 0)
                }
                _ => false,
            }
    }

    fn fields(&mut self, start: usize, end: usize, depth: usize) -> bool {
        let Some(count) = self.u16(start, end) else {
            return false;
        };
        let Ok(count) = u16::try_from(count) else {
            return false;
        };
        let mut choices: Vec<Choice> = Vec::new();
        let mut index = 0u16;
        let mut at = start + 2;
        loop {
            let next = if index == count {
                if at == end {
                    return true;
                }
                None
            } else {
                self.field(index, at, end, depth, &mut choices)
            };
            match next {
                Some(after) => {
                    at = after;
                    index += 1;
                }
                None => loop {
                    let Some(choice) = choices.pop() else {
                        return false;
                    };
                    self.wide.truncate(choice.mark);
                    if let Some(after) = self.value(choice.at, choice.kind, end, depth, true) {
                        at = after;
                        index = choice.index + 1;
                        break;
                    }
                },
            }
        }
    }

    fn field(
        &mut self,
        index: u16,
        at: usize,
        end: usize,
        depth: usize,
        choices: &mut Vec<Choice>,
    ) -> Option<usize> {
        self.step()?;
        let kind = self.byte(at.checked_add(4)?, end)?;
        let value_at = at + 5;
        if self.may_widen(value_at, kind, end) {
            choices.push(Choice {
                index,
                at: value_at,
                kind,
                mark: self.wide.len(),
            });
        }
        self.value(value_at, kind, end, depth, false)
    }

    fn value(
        &mut self,
        at: usize,
        kind: u8,
        end: usize,
        depth: usize,
        wide: bool,
    ) -> Option<usize> {
        self.step()?;
        if depth > MAX_FIELD_DEPTH {
            return None;
        }
        if kind == FIELD_HASH {
            if wide {
                self.wide.push(at);
            }
            return self.region(at, if wide { WIDE } else { NARROW }, end);
        }
        if let Some(size) = fixed_field_size(kind) {
            return self.region(at, size, end);
        }
        match kind {
            FIELD_STRING => {
                let len = self.u16(at, end)?;
                self.region(at + 2, len, end)
            }
            FIELD_LIST | FIELD_LIST2 => {
                let element = self.byte(at, end)?;
                let size = self.u32(at + 1, end)?;
                let body_end = self.region(at + 5, size, end)?;
                self.list(element, at + 5, body_end, depth)
                    .filter(|after| *after == body_end)
            }
            FIELD_POINTER | FIELD_EMBED => {
                if self.u32(at, end)? == 0 {
                    return Some(at + 4);
                }
                let size = self.u32(at + 4, end)?;
                let body_end = self.region(at + 8, size, end)?;
                self.fields(at + 8, body_end, depth + 1).then_some(body_end)
            }
            FIELD_OPTION => {
                let inner = self.byte(at, end)?;
                if self.byte(at + 1, end)? == 0 {
                    return Some(at + 2);
                }
                self.value(at + 2, inner, end, depth + 1, wide)
            }
            FIELD_MAP => {
                let key = self.byte(at, end)?;
                let value = self.byte(at + 1, end)?;
                let size = self.u32(at + 2, end)?;
                let body_end = self.region(at + 6, size, end)?;
                self.map(key, value, at + 6, body_end, depth)
            }
            _ => None,
        }
    }

    fn list(&mut self, element: u8, start: usize, end: usize, depth: usize) -> Option<usize> {
        let count = self.u32(start, end)?;
        let items = start + 4;
        if element == FIELD_HASH {
            let len = end - items;
            let width = match len.checked_div(count) {
                None if len == 0 => return Some(end),
                Some(NARROW) if len == NARROW * count => NARROW,
                Some(WIDE) if self.allow_wide && len == WIDE * count => WIDE,
                _ => return None,
            };
            if width == WIDE {
                self.wide.extend((0..count).map(|i| items + i * WIDE));
            }
            self.steps = self.steps.checked_sub(count)?;
            return Some(end);
        }
        let mut at = items;
        for _ in 0..count {
            at = self.value(at, element, end, depth + 1, false)?;
        }
        Some(at)
    }

    fn map(&mut self, key: u8, value: u8, start: usize, end: usize, depth: usize) -> Option<usize> {
        let mark = self.wide.len();
        let widths: &[(bool, bool)] = if !self.allow_wide {
            &[(false, false)]
        } else {
            match (key == FIELD_HASH, value == FIELD_HASH) {
                (false, false) => &[(false, false)],
                (true, false) => &[(false, false), (true, false)],
                (false, true) => &[(false, false), (false, true)],
                (true, true) => &[(false, false), (false, true), (true, false), (true, true)],
            }
        };
        let mut found: Option<Vec<usize>> = None;
        for &(wide_key, wide_value) in widths {
            let valid = self.entries(key, value, (wide_key, wide_value), start, end, depth);
            let record = self.wide.split_off(mark);
            if !valid {
                continue;
            }
            if found.is_some() {
                return None;
            }
            found = Some(record);
            if !wide_key && !wide_value {
                break;
            }
        }
        self.wide.extend(found?);
        Some(end)
    }

    fn entries(
        &mut self,
        key: u8,
        value: u8,
        (wide_key, wide_value): (bool, bool),
        start: usize,
        end: usize,
        depth: usize,
    ) -> bool {
        let Some(count) = self.u32(start, end) else {
            return false;
        };
        let mut at = start + 4;
        for _ in 0..count {
            let Some(after_key) = self.value(at, key, end, depth + 1, wide_key) else {
                return false;
            };
            let Some(after_value) = self.value(after_key, value, end, depth + 1, wide_value) else {
                return false;
            };
            at = after_value;
        }
        at == end
    }
}
