use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldValue<'a> {
    pub kind: u8,
    pub bytes: &'a [u8],
}

impl FieldValue<'_> {
    #[must_use]
    pub fn as_u32(&self) -> Option<u32> {
        matches!(self.kind, FIELD_U32 | 0x84 | 17 | 6)
            .then(|| self.bytes.get(..4))
            .flatten()
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
}

fn find_in_fields<'a>(
    cursor: &mut Cursor<'a>,
    path: &[u32],
) -> Result<Option<FieldValue<'a>>, WadError> {
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
        let bytes = cursor
            .data
            .get(start..cursor.at)
            .ok_or_else(|| WadError::InvalidProp("field value out of range".into()))?;
        if rest.is_empty() {
            return Ok(Some(FieldValue { kind, bytes }));
        }
        if matches!(kind, FIELD_POINTER | FIELD_EMBED) && bytes.len() > 8 {
            let mut inner = Cursor { data: bytes, at: 8 };
            return find_in_fields(&mut inner, rest);
        }
        return Ok(None);
    }
    Ok(None)
}
pub fn field_value<'a>(body: &'a [u8], path: &[u32]) -> Result<Option<FieldValue<'a>>, WadError> {
    find_in_fields(&mut Cursor { data: body, at: 0 }, path)
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
    let found = {
        let mut cursor = Cursor { data: body, at: 0 };
        let count = cursor.u16("field count")?;
        let mut found = None;
        for _ in 0..count {
            let name = cursor.u32("field name")?;
            let kind = cursor.take(1, "field type")?[0];
            if name == field_hash && kinds.contains(&kind) {
                found = Some(cursor.at);
                break;
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
