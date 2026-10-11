use super::*;

fn reference_offsets_in_value(
    cursor: &mut Cursor<'_>,
    kind: u8,
    depth: usize,
    out: &mut Vec<usize>,
) -> Result<(), WadError> {
    if depth > MAX_FIELD_DEPTH {
        return Err(WadError::InvalidProp(format!(
            "fields nested deeper than {MAX_FIELD_DEPTH}"
        )));
    }
    match kind {
        FIELD_HASH | FIELD_LINK => {
            let at = cursor.at;
            cursor.take(4, "reference")?;
            out.push(at);
        }
        FIELD_LIST | FIELD_LIST2 => {
            let element = cursor.take(1, "list element type")?[0];
            cursor.u32("list size")?;
            let count = cursor.count("list count", &[element])?;
            for _ in 0..count {
                reference_offsets_in_value(cursor, element, depth + 1, out)?;
            }
        }
        FIELD_POINTER | FIELD_EMBED => {
            if cursor.u32("class")? != 0 {
                cursor.u32("struct size")?;
                reference_offsets_in_fields(cursor, depth + 1, out)?;
            }
        }
        FIELD_OPTION => {
            let inner = cursor.take(1, "option type")?[0];
            if cursor.take(1, "option count")?[0] != 0 {
                reference_offsets_in_value(cursor, inner, depth + 1, out)?;
            }
        }
        FIELD_MAP => {
            let key_kind = cursor.take(1, "map key type")?[0];
            let value_kind = cursor.take(1, "map value type")?[0];
            cursor.u32("map size")?;
            let count = cursor.count("map count", &[key_kind, value_kind])?;
            for _ in 0..count {
                if key_kind == FIELD_LINK {
                    out.push(cursor.at);
                }
                skip_field_value(cursor, key_kind)?;
                reference_offsets_in_value(cursor, value_kind, depth + 1, out)?;
            }
        }
        _ => skip_field_value(cursor, kind)?,
    }
    Ok(())
}

fn reference_offsets_in_fields(
    cursor: &mut Cursor<'_>,
    depth: usize,
    out: &mut Vec<usize>,
) -> Result<(), WadError> {
    let count = cursor.u16("field count")?;
    for _ in 0..count {
        cursor.u32("field name")?;
        let kind = cursor.take(1, "field type")?[0];
        reference_offsets_in_value(cursor, kind, depth, out)?;
    }
    Ok(())
}

fn reference_offsets(body: &[u8]) -> Result<Vec<usize>, WadError> {
    let mut out = Vec::new();
    reference_offsets_in_fields(&mut Cursor { data: body, at: 0 }, 0, &mut out)?;
    Ok(out)
}

fn u32_at(body: &[u8], at: usize) -> Result<u32, WadError> {
    body.get(at..at + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or_else(|| WadError::InvalidProp(format!("truncated reference at offset {at}")))
}

pub fn reference_values(body: &[u8]) -> Result<Vec<u32>, WadError> {
    reference_offsets(body)?
        .into_iter()
        .map(|at| u32_at(body, at))
        .collect()
}

pub fn remap_references(
    body: &mut [u8],
    map: &std::collections::BTreeMap<u32, u32>,
) -> Result<usize, WadError> {
    let offsets = reference_offsets(body)?;
    let mut changed = 0;
    for at in offsets {
        let Some(&target) = map.get(&u32_at(body, at)?) else {
            continue;
        };
        let slot = body
            .get_mut(at..at + 4)
            .ok_or_else(|| WadError::InvalidProp(format!("truncated reference at offset {at}")))?;
        slot.copy_from_slice(&target.to_le_bytes());
        changed += 1;
    }
    Ok(changed)
}
