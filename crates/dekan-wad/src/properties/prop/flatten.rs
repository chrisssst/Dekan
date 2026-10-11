use super::*;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct FlatField {
    pub path: String,
    pub kind: u8,
    pub hex: String,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn flatten_value(
    cursor: &mut Cursor<'_>,
    kind: u8,
    path: &str,
    depth: usize,
    out: &mut Vec<FlatField>,
) -> Result<(), WadError> {
    if depth > MAX_FIELD_DEPTH {
        return Err(WadError::InvalidProp(format!(
            "fields nested deeper than {MAX_FIELD_DEPTH}"
        )));
    }
    match kind {
        FIELD_LIST | FIELD_LIST2 => {
            let element = cursor.take(1, "list element type")?[0];
            cursor.u32("list size")?;
            let count = cursor.count("list count", &[element])?;
            out.push(FlatField {
                path: format!("{path}.len"),
                kind,
                hex: format!("{count:08x}"),
            });
            for i in 0..count {
                flatten_value(cursor, element, &format!("{path}[{i}]"), depth + 1, out)?;
            }
        }
        FIELD_POINTER | FIELD_EMBED => {
            let class = cursor.u32("class")?;
            out.push(FlatField {
                path: format!("{path}.class"),
                kind,
                hex: format!("{class:08x}"),
            });
            if class != 0 {
                cursor.u32("struct size")?;
                flatten_fields_at(cursor, path, depth + 1, out)?;
            }
        }
        FIELD_OPTION => {
            let inner = cursor.take(1, "option type")?[0];
            let present = cursor.take(1, "option count")?[0];
            out.push(FlatField {
                path: format!("{path}.some"),
                kind,
                hex: format!("{present:02x}"),
            });
            if present != 0 {
                flatten_value(cursor, inner, &format!("{path}?"), depth + 1, out)?;
            }
        }
        FIELD_MAP => {
            let key_kind = cursor.take(1, "map key type")?[0];
            let value_kind = cursor.take(1, "map value type")?[0];
            cursor.u32("map size")?;
            let count = cursor.count("map count", &[key_kind, value_kind])?;
            out.push(FlatField {
                path: format!("{path}.len"),
                kind,
                hex: format!("{count:08x}"),
            });
            for _ in 0..count {
                let start = cursor.at;
                skip_field_value(cursor, key_kind)?;
                let key = cursor
                    .data
                    .get(start..cursor.at)
                    .ok_or_else(|| WadError::InvalidProp("map key out of range".into()))?;
                flatten_value(
                    cursor,
                    value_kind,
                    &format!("{path}{{{}}}", hex(key)),
                    depth + 1,
                    out,
                )?;
            }
        }
        _ => {
            let start = cursor.at;
            skip_field_value(cursor, kind)?;
            let bytes = cursor
                .data
                .get(start..cursor.at)
                .ok_or_else(|| WadError::InvalidProp("field value out of range".into()))?;
            out.push(FlatField {
                path: path.to_owned(),
                kind,
                hex: hex(bytes),
            });
        }
    }
    Ok(())
}

fn flatten_fields_at(
    cursor: &mut Cursor<'_>,
    prefix: &str,
    depth: usize,
    out: &mut Vec<FlatField>,
) -> Result<(), WadError> {
    let count = cursor.u16("field count")?;
    for _ in 0..count {
        let name = cursor.u32("field name")?;
        let kind = cursor.take(1, "field type")?[0];
        let path = if prefix.is_empty() {
            format!("{name:08x}")
        } else {
            format!("{prefix}/{name:08x}")
        };
        flatten_value(cursor, kind, &path, depth, out)?;
    }
    Ok(())
}

pub fn flatten_fields(body: &[u8]) -> Result<Vec<FlatField>, WadError> {
    let mut out = Vec::new();
    flatten_fields_at(&mut Cursor { data: body, at: 0 }, "", 0, &mut out)?;
    Ok(out)
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct FieldChange {
    pub path: String,
    pub before: Option<String>,
    pub after: Option<String>,
}

pub fn diff_fields(before: &[u8], after: &[u8]) -> Result<Vec<FieldChange>, WadError> {
    let left: std::collections::BTreeMap<String, String> = flatten_fields(before)?
        .into_iter()
        .map(|f| (f.path, f.hex))
        .collect();
    let right: std::collections::BTreeMap<String, String> = flatten_fields(after)?
        .into_iter()
        .map(|f| (f.path, f.hex))
        .collect();
    let paths: std::collections::BTreeSet<&String> = left.keys().chain(right.keys()).collect();
    Ok(paths
        .into_iter()
        .filter(|p| left.get(*p) != right.get(*p))
        .map(|p| FieldChange {
            path: p.clone(),
            before: left.get(p).cloned(),
            after: right.get(p).cloned(),
        })
        .collect())
}
