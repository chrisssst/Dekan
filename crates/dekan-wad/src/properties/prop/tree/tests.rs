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
