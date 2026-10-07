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

pub const FIELD_FILE: u8 = 18;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Shape {
    kind: u8,
    inner: Option<u8>,
}

impl Shape {
    fn of(value: &Value) -> Self {
        match value {
            Value::Raw { kind, .. } | Value::Struct { kind, .. } => Self {
                kind: *kind,
                inner: None,
            },
            Value::List { kind, element, .. } => Self {
                kind: *kind,
                inner: Some(*element),
            },
            Value::Optional { inner, .. } => Self {
                kind: FIELD_OPTION,
                inner: Some(*inner),
            },
            Value::Map { value, .. } => Self {
                kind: FIELD_MAP,
                inner: Some(*value),
            },
        }
    }
}

#[derive(Debug, Default)]
pub struct FieldShapes(std::collections::HashMap<(u32, u32), Shape>);

impl FieldShapes {
    pub fn record(&mut self, class: u32, fields: &[Field]) {
        for field in fields {
            self.0
                .entry((class, field.name))
                .or_insert_with(|| Shape::of(&field.value));
            self.record_nested(&field.value);
        }
    }

    fn record_nested(&mut self, value: &Value) {
        match value {
            Value::Struct { class, fields, .. } => self.record(*class, fields),
            Value::List { items, .. } => items.iter().for_each(|item| self.record_nested(item)),
            Value::Optional {
                value: Some(inner), ..
            } => self.record_nested(inner),
            Value::Map { entries, .. } => entries.iter().for_each(|(k, v)| {
                self.record_nested(k);
                self.record_nested(v);
            }),
            Value::Raw { .. } | Value::Optional { value: None, .. } => {}
        }
    }

    pub fn strings_to_files(&self, class: u32, fields: &mut [Field]) -> usize {
        fields
            .iter_mut()
            .map(|field| {
                let expected = self.0.get(&(class, field.name)).copied();
                expected.map_or(0, |shape| retype(&mut field.value, shape))
                    + self.strings_to_files_nested(&mut field.value)
            })
            .sum()
    }

    fn strings_to_files_nested(&self, value: &mut Value) -> usize {
        match value {
            Value::Struct { class, fields, .. } => self.strings_to_files(*class, fields),
            Value::List { items, .. } => items
                .iter_mut()
                .map(|item| self.strings_to_files_nested(item))
                .sum(),
            Value::Optional {
                value: Some(inner), ..
            } => self.strings_to_files_nested(inner),
            Value::Map { entries, .. } => entries
                .iter_mut()
                .map(|(k, v)| self.strings_to_files_nested(k) + self.strings_to_files_nested(v))
                .sum(),
            Value::Raw { .. } | Value::Optional { value: None, .. } => 0,
        }
    }
}

fn retype(value: &mut Value, expected: Shape) -> usize {
    let wants_file = |kind: u8| kind == FIELD_STRING && expected.inner == Some(FIELD_FILE);
    match value {
        Value::Raw { kind, bytes } if *kind == FIELD_STRING && expected.kind == FIELD_FILE => {
            *value = file_from_string(bytes);
            1
        }
        Value::Optional { inner, value } if expected.kind == FIELD_OPTION && wants_file(*inner) => {
            *inner = FIELD_FILE;
            if let Some(present) = value {
                if let Value::Raw { bytes, .. } = present.as_ref() {
                    **present = file_from_string(bytes);
                }
            }
            1
        }
        Value::List {
            kind,
            element,
            items,
        } if *kind == expected.kind && wants_file(*element) => {
            *element = FIELD_FILE;
            for item in items.iter_mut() {
                if let Value::Raw { bytes, .. } = item {
                    *item = file_from_string(bytes);
                }
            }
            1
        }
        _ => 0,
    }
}

fn file_from_string(encoded: &[u8]) -> Value {
    let path = encoded.get(2..).unwrap_or_default().to_ascii_lowercase();
    Value::Raw {
        kind: FIELD_FILE,
        bytes: xxhash_rust::xxh64::xxh64(&path, 0).to_le_bytes().to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(kind: u8, bytes: &[u8]) -> Value {
        Value::Raw {
            kind,
            bytes: bytes.to_vec(),
        }
    }

    fn text(value: &str) -> Value {
        let mut bytes = u16::try_from(value.len())
            .expect("short")
            .to_le_bytes()
            .to_vec();
        bytes.extend_from_slice(value.as_bytes());
        raw(FIELD_STRING, &bytes)
    }

    fn file(value: &str) -> Value {
        raw(
            FIELD_FILE,
            &xxhash_rust::xxh64::xxh64(value.to_ascii_lowercase().as_bytes(), 0).to_le_bytes(),
        )
    }

    fn character(texture: Value, icon: Value, clips: Value, name: Value) -> Vec<Field> {
        vec![
            Field {
                name: 1,
                value: texture,
            },
            Field {
                name: 2,
                value: icon,
            },
            Field {
                name: 3,
                value: Value::Struct {
                    kind: FIELD_EMBED,
                    class: 0xB0B0,
                    fields: vec![Field {
                        name: 4,
                        value: clips,
                    }],
                },
            },
            Field {
                name: 5,
                value: name,
            },
        ]
    }

    fn optional(inner: u8, value: Value) -> Value {
        Value::Optional {
            inner,
            value: Some(Box::new(value)),
        }
    }

    fn list(element: u8, items: Vec<Value>) -> Value {
        Value::List {
            kind: FIELD_LIST,
            element,
            items,
        }
    }

    #[test]
    fn a_count_larger_than_the_bytes_left_is_refused_even_for_empty_items() {
        let mut body = vec![1, 0];
        body.extend_from_slice(&7u32.to_le_bytes());
        body.push(FIELD_LIST);
        body.push(0);
        body.extend_from_slice(&4u32.to_le_bytes());
        body.extend_from_slice(&0x0002_0000u32.to_le_bytes());
        assert!(parse_fields(&body).is_err());
        assert!(super::super::flatten_fields(&body).is_err());
        assert!(super::super::reference_values(&body).is_err());

        let mut map = vec![1, 0];
        map.extend_from_slice(&7u32.to_le_bytes());
        map.extend_from_slice(&[FIELD_MAP, 0, 0]);
        map.extend_from_slice(&4u32.to_le_bytes());
        map.extend_from_slice(&u32::MAX.to_le_bytes());
        assert!(parse_fields(&map).is_err());
    }

    #[test]
    fn stale_text_paths_become_the_file_references_the_game_declares() {
        let mut shapes = FieldShapes::default();
        shapes.record(
            0xC1A5,
            &character(
                file("ASSETS/a.tex"),
                optional(FIELD_FILE, file("ASSETS/i.dds")),
                list(FIELD_FILE, vec![file("ASSETS/c.anm")]),
                text("Varus"),
            ),
        );
        let mut stale = character(
            text("ASSETS/Sniper/A.tex"),
            optional(FIELD_STRING, text("ASSETS/Sniper/I.dds")),
            list(
                FIELD_STRING,
                vec![text("ASSETS/Sniper/C.anm"), text("ASSETS/Sniper/D.anm")],
            ),
            text("Sniper Varus"),
        );

        assert_eq!(shapes.strings_to_files(0xC1A5, &mut stale), 3);
        assert_eq!(
            stale,
            character(
                file("assets/sniper/a.tex"),
                optional(FIELD_FILE, file("assets/sniper/i.dds")),
                list(
                    FIELD_FILE,
                    vec![file("assets/sniper/c.anm"), file("assets/sniper/d.anm")]
                ),
                text("Sniper Varus"),
            ),
            "only fields the game declares as files change; text stays text"
        );
        assert_eq!(
            shapes.strings_to_files(0xC1A5, &mut stale),
            0,
            "a second pass changes nothing"
        );
        assert!(write_fields(&stale).is_ok());
    }

    #[test]
    fn an_unknown_class_or_an_empty_optional_is_left_alone() {
        let mut shapes = FieldShapes::default();
        shapes.record(
            0xC1A5,
            &[Field {
                name: 2,
                value: Value::Optional {
                    inner: FIELD_FILE,
                    value: None,
                },
            }],
        );
        let mut other_class = vec![Field {
            name: 2,
            value: text("ASSETS/x.dds"),
        }];
        assert_eq!(shapes.strings_to_files(0xD00D, &mut other_class), 0);

        let mut empty = vec![Field {
            name: 2,
            value: Value::Optional {
                inner: FIELD_STRING,
                value: None,
            },
        }];
        assert_eq!(shapes.strings_to_files(0xC1A5, &mut empty), 1);
        assert_eq!(
            empty[0].value,
            Value::Optional {
                inner: FIELD_FILE,
                value: None
            },
            "an empty optional is redeclared with the game's inner type"
        );
    }

    fn sample() -> Vec<Field> {
        let inner = vec![
            Field {
                name: 1,
                value: raw(7, &5u32.to_le_bytes()),
            },
            Field {
                name: 2,
                value: raw(FIELD_STRING, &[2, 0, b'o', b'k']),
            },
        ];
        vec![
            Field {
                name: 10,
                value: Value::Struct {
                    kind: FIELD_EMBED,
                    class: 0xAAAA,
                    fields: inner.clone(),
                },
            },
            Field {
                name: 11,
                value: Value::Struct {
                    kind: FIELD_POINTER,
                    class: 0,
                    fields: Vec::new(),
                },
            },
            Field {
                name: 12,
                value: Value::List {
                    kind: FIELD_LIST2,
                    element: FIELD_POINTER,
                    items: vec![Value::Struct {
                        kind: FIELD_POINTER,
                        class: 0xBBBB,
                        fields: inner,
                    }],
                },
            },
            Field {
                name: 13,
                value: Value::Optional {
                    inner: 0x84,
                    value: Some(Box::new(raw(0x84, &9u32.to_le_bytes()))),
                },
            },
            Field {
                name: 14,
                value: Value::Map {
                    key: 17,
                    value: 0x84,
                    entries: vec![(raw(17, &1u32.to_le_bytes()), raw(0x84, &2u32.to_le_bytes()))],
                },
            },
            Field {
                name: 15,
                value: Value::Optional {
                    inner: 7,
                    value: None,
                },
            },
        ]
    }

    #[test]
    fn test_a_body_written_and_read_back_is_identical() {
        let bytes = write_fields(&sample()).expect("write");
        let parsed = parse_fields(&bytes).expect("parse");
        assert_eq!(parsed, sample());
        assert_eq!(write_fields(&parsed).expect("rewrite"), bytes);
        assert_eq!(
            crate::prop::flatten_fields(&bytes)
                .expect("the walker reads it too")
                .len(),
            crate::prop::flatten_fields(&write_fields(&parsed).expect("rewrite"))
                .expect("again")
                .len()
        );
    }

    #[test]
    fn test_an_edit_that_changes_sizes_recomputes_every_enclosing_size() {
        let mut fields = sample();
        let list = field_mut(&mut fields, 12)
            .and_then(Value::items_mut)
            .expect("list");
        let first = list[0].clone();
        list.push(first);
        let embed = field_mut(&mut fields, 10)
            .and_then(Value::fields_mut)
            .expect("embed");
        set_field(
            embed,
            3,
            raw(FIELD_STRING, &[5, 0, b'h', b'e', b'l', b'l', b'o']),
        );
        assert!(remove_field(&mut fields, 15).is_some());
        let bytes = write_fields(&fields).expect("write");
        assert_eq!(parse_fields(&bytes).expect("parse"), fields);
        assert!(
            crate::prop::flatten_fields(&bytes).is_ok(),
            "sizes agree with the walker"
        );
    }

    #[test]
    fn test_a_damaged_body_is_an_error_not_a_panic() {
        let bytes = write_fields(&sample()).expect("write");
        for cut in 0..bytes.len() {
            assert!(parse_fields(&bytes[..cut]).is_err(), "cut {cut}");
        }
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(
            parse_fields(&trailing).is_err(),
            "trailing bytes are refused"
        );
        let mut wrong_size = bytes;
        let embed_size_at = 2 + 4 + 1 + 4;
        wrong_size[embed_size_at] = wrong_size[embed_size_at].wrapping_add(1);
        assert!(
            parse_fields(&wrong_size).is_err(),
            "a size that disagrees with the fields is refused"
        );
    }

    #[test]
    fn test_nesting_is_bounded() {
        let mut deep = 1u16.to_le_bytes().to_vec();
        deep.extend_from_slice(&1u32.to_le_bytes());
        deep.push(FIELD_OPTION);
        for _ in 0..80 {
            deep.extend_from_slice(&[FIELD_OPTION, 1]);
        }
        deep.extend_from_slice(&[7, 1, 7, 0, 0, 0]);
        assert!(parse_fields(&deep).is_err());
    }
}
