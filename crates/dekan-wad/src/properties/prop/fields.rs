use tracing::{debug, warn};

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldValue<'a> {
    pub kind: u8,
    pub bytes: &'a [u8],
}

impl FieldValue<'_> {
    #[must_use]
    pub fn as_u32(&self) -> Option<u32> {
        let b: [u8; 4] = self.bytes.try_into().ok()?;
        matches!(self.kind, FIELD_U32 | 0x84 | 17 | 6).then(|| u32::from_le_bytes(b))
    }
}

struct Found {
    kind: u8,
    start: usize,
    end: usize,
}

fn find_in_fields(cursor: &mut Cursor<'_>, path: &[u32]) -> Result<Option<Found>, WadError> {
    let Some((&wanted, rest)) = path.split_first() else {
        return Ok(None);
    };
    let count = cursor.u16("field count")?;
    for _ in 0..count {
        let name = cursor.u32("field name")?;
        let kind = cursor.take(1, "field type")?[0];
        let start = cursor.at;
        skip_field_value(cursor, kind)?;
        if name != wanted {
            continue;
        }
        let end = cursor.at;
        if rest.is_empty() {
            return Ok(Some(Found { kind, start, end }));
        }
        if matches!(kind, FIELD_POINTER | FIELD_EMBED) && end - start > 8 {
            cursor.at = start + 8;
            return find_in_fields(cursor, rest);
        }
        debug!(
            field = format!("{wanted:08x}"),
            kind, "Field path crosses a value that is not a struct"
        );
        return Ok(None);
    }
    Ok(None)
}

pub fn field_value<'a>(body: &'a [u8], path: &[u32]) -> Result<Option<FieldValue<'a>>, WadError> {
    let wide = tree::hash_widths(body)?;
    let Some(found) = find_in_fields(&mut Cursor::new(body, &wide), path)? else {
        return Ok(None);
    };
    let bytes = body
        .get(found.start..found.end)
        .ok_or_else(|| WadError::InvalidProp("field value out of range".into()))?;
    Ok(Some(FieldValue {
        kind: found.kind,
        bytes,
    }))
}

pub fn set_int_field(body: &mut [u8], field_hash: u32, value: u32) -> Result<bool, WadError> {
    set_top_level_int(body, field_hash, value, &[FIELD_I32, FIELD_U32])
}

fn set_top_level_int(
    body: &mut [u8],
    field_hash: u32,
    value: u32,
    kinds: &[u8],
) -> Result<bool, WadError> {
    let wide = tree::hash_widths(body)?;
    let found = {
        let mut cursor = Cursor::new(body, &wide);
        let count = cursor.u16("field count")?;
        let mut found = None;
        for _ in 0..count {
            let name = cursor.u32("field name")?;
            let kind = cursor.take(1, "field type")?[0];
            if name == field_hash {
                if kinds.contains(&kind) {
                    found = Some(cursor.at);
                    break;
                }
                warn!(
                    field = format!("{field_hash:08x}"),
                    kind, "Integer field has an unexpected type; left unchanged"
                );
                return Ok(false);
            }
            skip_field_value(&mut cursor, kind)?;
        }
        found
    };
    let Some(at) = found else {
        return Ok(false);
    };
    let slot = body
        .get_mut(at..at + 4)
        .ok_or_else(|| WadError::InvalidProp(format!("truncated u32 field at offset {at}")))?;
    slot.copy_from_slice(&value.to_le_bytes());
    Ok(true)
}
