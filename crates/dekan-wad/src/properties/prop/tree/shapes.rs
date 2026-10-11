use super::*;

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
