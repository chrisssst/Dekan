use super::{
    Cursor, FIELD_EMBED, FIELD_LIST, FIELD_LIST2, FIELD_MAP, FIELD_OPTION, FIELD_POINTER,
    FIELD_STRING, MAX_FIELD_DEPTH, fixed_field_size,
};
use crate::error::WadError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Raw {
        kind: u8,
        bytes: Vec<u8>,
    },
    List {
        kind: u8,
        element: u8,
        items: Vec<Value>,
    },
    Struct {
        kind: u8,
        class: u32,
        fields: Vec<Field>,
    },
    Optional {
        inner: u8,
        value: Option<Box<Value>>,
    },
    Map {
        key: u8,
        value: u8,
        entries: Vec<(Value, Value)>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub name: u32,
    pub value: Value,
}

impl Value {
    #[must_use]
    pub fn kind(&self) -> u8 {
        match self {
            Self::Raw { kind, .. } | Self::List { kind, .. } | Self::Struct { kind, .. } => *kind,
            Self::Optional { .. } => FIELD_OPTION,
            Self::Map { .. } => FIELD_MAP,
        }
    }

    #[must_use]
    pub fn as_u32(&self) -> Option<u32> {
        match self {
            Self::Raw { bytes, .. } if bytes.len() == 4 => {
                Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
            }
            _ => None,
        }
    }

    #[must_use]
    pub fn fields(&self) -> Option<&[Field]> {
        match self {
            Self::Struct { fields, .. } => Some(fields),
            _ => None,
        }
    }

    pub fn fields_mut(&mut self) -> Option<&mut Vec<Field>> {
        match self {
            Self::Struct { fields, .. } => Some(fields),
            _ => None,
        }
    }

    #[must_use]
    pub fn class(&self) -> Option<u32> {
        match self {
            Self::Struct { class, .. } => Some(*class),
            _ => None,
        }
    }

    #[must_use]
    pub fn items(&self) -> Option<&[Value]> {
        match self {
            Self::List { items, .. } => Some(items),
            _ => None,
        }
    }

    pub fn items_mut(&mut self) -> Option<&mut Vec<Value>> {
        match self {
            Self::List { items, .. } => Some(items),
            _ => None,
        }
    }
}

#[must_use]
pub fn field(fields: &[Field], name: u32) -> Option<&Value> {
    fields.iter().find(|f| f.name == name).map(|f| &f.value)
}

pub fn field_mut(fields: &mut [Field], name: u32) -> Option<&mut Value> {
    fields
        .iter_mut()
        .find(|f| f.name == name)
        .map(|f| &mut f.value)
}

pub fn set_field(fields: &mut Vec<Field>, name: u32, value: Value) {
    match fields.iter_mut().find(|f| f.name == name) {
        Some(existing) => existing.value = value,
        None => fields.push(Field { name, value }),
    }
}

pub fn remove_field(fields: &mut Vec<Field>, name: u32) -> Option<Value> {
    let at = fields.iter().position(|f| f.name == name)?;
    Some(fields.remove(at).value)
}

fn depth_error() -> WadError {
    WadError::InvalidProp(format!("fields nested deeper than {MAX_FIELD_DEPTH}"))
}

fn read_value(cursor: &mut Cursor<'_>, kind: u8, depth: usize) -> Result<Value, WadError> {
    if depth > MAX_FIELD_DEPTH {
        return Err(depth_error());
    }
    if let Some(size) = fixed_field_size(kind) {
        return Ok(Value::Raw {
            kind,
            bytes: cursor.take(size, "field value")?.to_vec(),
        });
    }
    match kind {
        FIELD_STRING => {
            let start = cursor.at;
            let len = usize::from(cursor.u16("string length")?);
            cursor.take(len, "string")?;
            Ok(Value::Raw {
                kind,
                bytes: cursor.data[start..cursor.at].to_vec(),
            })
        }
        FIELD_LIST | FIELD_LIST2 => {
            let element = cursor.take(1, "list element type")?[0];
            let size = cursor.u32("list size")? as usize;
            let end = checked_end(cursor, size, "list")?;
            let count = cursor.count("list count", &[element])?;
            let mut items = Vec::with_capacity((count as usize).min(size));
            for _ in 0..count {
                items.push(read_value(cursor, element, depth + 1)?);
            }
            expect_end(cursor, end, "list")?;
            Ok(Value::List {
                kind,
                element,
                items,
            })
        }
        FIELD_POINTER | FIELD_EMBED => {
            let class = cursor.u32("class")?;
            if class == 0 {
                return Ok(Value::Struct {
                    kind,
                    class,
                    fields: Vec::new(),
                });
            }
            let size = cursor.u32("struct size")? as usize;
            let end = checked_end(cursor, size, "struct")?;
            let fields = read_fields(cursor, depth + 1)?;
            expect_end(cursor, end, "struct")?;
            Ok(Value::Struct {
                kind,
                class,
                fields,
            })
        }
        FIELD_OPTION => {
            let inner = cursor.take(1, "option type")?[0];
            let present = cursor.take(1, "option count")?[0];
            let value = if present == 0 {
                None
            } else {
                Some(Box::new(read_value(cursor, inner, depth + 1)?))
            };
            Ok(Value::Optional { inner, value })
        }
        FIELD_MAP => {
            let key = cursor.take(1, "map key type")?[0];
            let value = cursor.take(1, "map value type")?[0];
            let size = cursor.u32("map size")? as usize;
            let end = checked_end(cursor, size, "map")?;
            let count = cursor.count("map count", &[key, value])?;
            let mut entries = Vec::with_capacity((count as usize).min(size));
            for _ in 0..count {
                let k = read_value(cursor, key, depth + 1)?;
                let v = read_value(cursor, value, depth + 1)?;
                entries.push((k, v));
            }
            expect_end(cursor, end, "map")?;
            Ok(Value::Map {
                key,
                value,
                entries,
            })
        }
        other => Err(WadError::InvalidProp(format!(
            "unknown field type {other:#04x} at offset {}",
            cursor.at
        ))),
    }
}

fn checked_end(cursor: &Cursor<'_>, size: usize, what: &str) -> Result<usize, WadError> {
    let end = cursor
        .at
        .checked_add(size)
        .ok_or_else(|| WadError::InvalidProp(format!("{what}: size overflow")))?;
    if end > cursor.data.len() {
        return Err(WadError::InvalidProp(format!(
            "{what} of {size} bytes at offset {} runs past the body",
            cursor.at
        )));
    }
    Ok(end)
}

fn expect_end(cursor: &Cursor<'_>, end: usize, what: &str) -> Result<(), WadError> {
    if cursor.at == end {
        Ok(())
    } else {
        Err(WadError::InvalidProp(format!(
            "{what} ends at offset {} but declared {end}",
            cursor.at
        )))
    }
}

fn read_fields(cursor: &mut Cursor<'_>, depth: usize) -> Result<Vec<Field>, WadError> {
    let count = cursor.u16("field count")?;
    let mut fields = Vec::with_capacity(usize::from(count));
    for _ in 0..count {
        let name = cursor.u32("field name")?;
        let kind = cursor.take(1, "field type")?[0];
        fields.push(Field {
            name,
            value: read_value(cursor, kind, depth)?,
        });
    }
    Ok(fields)
}

pub fn parse_fields(body: &[u8]) -> Result<Vec<Field>, WadError> {
    let mut cursor = Cursor { data: body, at: 0 };
    let fields = read_fields(&mut cursor, 0)?;
    if cursor.at != body.len() {
        return Err(WadError::InvalidProp(format!(
            "{} bytes after the last field",
            body.len() - cursor.at
        )));
    }
    Ok(fields)
}

fn write_sized(
    out: &mut Vec<u8>,
    write: impl FnOnce(&mut Vec<u8>) -> Result<(), WadError>,
) -> Result<(), WadError> {
    let at = out.len();
    out.extend_from_slice(&[0; 4]);
    write(out)?;
    let size = u32::try_from(out.len() - at - 4)
        .map_err(|_| WadError::InvalidProp("value larger than 4 GiB".into()))?;
    out[at..at + 4].copy_from_slice(&size.to_le_bytes());
    Ok(())
}

fn count_u32(len: usize) -> Result<u32, WadError> {
    u32::try_from(len).map_err(|_| WadError::InvalidProp("too many items".into()))
}

fn write_value(out: &mut Vec<u8>, value: &Value) -> Result<(), WadError> {
    match value {
        Value::Raw { bytes, .. } => out.extend_from_slice(bytes),
        Value::List { element, items, .. } => {
            out.push(*element);
            write_sized(out, |out| {
                out.extend_from_slice(&count_u32(items.len())?.to_le_bytes());
                items.iter().try_for_each(|item| write_value(out, item))
            })?;
        }
        Value::Struct { class, fields, .. } => {
            out.extend_from_slice(&class.to_le_bytes());
            if *class != 0 {
                write_sized(out, |out| write_field_list(out, fields))?;
            }
        }
        Value::Optional { inner, value } => {
            out.push(*inner);
            match value {
                Some(value) => {
                    out.push(1);
                    write_value(out, value)?;
                }
                None => out.push(0),
            }
        }
        Value::Map {
            key,
            value,
            entries,
        } => {
            out.push(*key);
            out.push(*value);
            write_sized(out, |out| {
                out.extend_from_slice(&count_u32(entries.len())?.to_le_bytes());
                entries.iter().try_for_each(|(k, v)| {
                    write_value(out, k)?;
                    write_value(out, v)
                })
            })?;
        }
    }
    Ok(())
}

fn write_field_list(out: &mut Vec<u8>, fields: &[Field]) -> Result<(), WadError> {
    let count = u16::try_from(fields.len())
        .map_err(|_| WadError::InvalidProp("more than 65 535 fields".into()))?;
    out.extend_from_slice(&count.to_le_bytes());
    for field in fields {
        out.extend_from_slice(&field.name.to_le_bytes());
        out.push(field.value.kind());
        write_value(out, &field.value)?;
    }
    Ok(())
}

pub fn write_fields(fields: &[Field]) -> Result<Vec<u8>, WadError> {
    let mut out = Vec::new();
    write_field_list(&mut out, fields)?;
    Ok(out)
}

mod shapes;

pub use shapes::{FIELD_FILE, FieldShapes};

#[cfg(test)]
mod tests;
