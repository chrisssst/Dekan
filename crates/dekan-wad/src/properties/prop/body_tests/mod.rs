use super::*;

fn field(body: &mut Vec<u8>, name: u32, kind: u8, value: &[u8]) {
    body.extend_from_slice(&name.to_le_bytes());
    body.push(kind);
    body.extend_from_slice(value);
}

fn body_with_target_after_every_container() -> Vec<u8> {
    let mut body = 8u16.to_le_bytes().to_vec();
    let mut string = 3u16.to_le_bytes().to_vec();
    string.extend_from_slice(b"abc");
    field(&mut body, 1, FIELD_STRING, &string);
    let mut list = vec![FIELD_U32];
    list.extend_from_slice(&12u32.to_le_bytes());
    list.extend_from_slice(&2u32.to_le_bytes());
    list.extend_from_slice(&[0xEE; 8]);
    field(&mut body, 2, FIELD_LIST, &list);
    let mut embed = 0xAAAA_AAAAu32.to_le_bytes().to_vec();
    embed.extend_from_slice(&8u32.to_le_bytes());
    embed.extend_from_slice(&1u16.to_le_bytes());
    embed.extend_from_slice(&0x8722_5880u32.to_le_bytes());
    embed.push(1);
    embed.push(1);
    field(&mut body, 3, FIELD_EMBED, &embed);
    field(&mut body, 4, FIELD_POINTER, &0u32.to_le_bytes());
    field(&mut body, 5, FIELD_OPTION, &[FIELD_U32, 1, 9, 9, 9, 9]);
    let mut map = vec![FIELD_U32, FIELD_U32];
    map.extend_from_slice(&12u32.to_le_bytes());
    map.extend_from_slice(&1u32.to_le_bytes());
    map.extend_from_slice(&[0xDD; 8]);
    field(&mut body, 6, FIELD_MAP, &map);
    field(&mut body, 7, 13, &[0xCC; 16]);
    field(&mut body, 0x8722_5880, FIELD_U32, &2u32.to_le_bytes());
    body
}

#[test]
fn test_a_top_level_u32_field_is_found_after_every_container_kind() {
    let mut body = body_with_target_after_every_container();
    let original = body.clone();
    assert!(set_int_field(&mut body, 0x8722_5880, 1).expect("set"));
    let len = body.len();
    assert_eq!(&body[len - 4..], &1u32.to_le_bytes());
    assert_eq!(
        &body[..len - 4],
        &original[..len - 4],
        "nothing else moves, not even the same hash nested inside the embed"
    );
}

#[test]
fn test_a_missing_field_changes_nothing() {
    let mut body = body_with_target_after_every_container();
    let original = body.clone();
    assert!(!set_int_field(&mut body, 0x1234_5678, 1).expect("walk"));
    assert_eq!(body, original);
}

#[test]
fn test_every_truncation_of_a_body_is_an_error_or_leaves_it_untouched() {
    let full = body_with_target_after_every_container();
    for cut in 0..full.len() - 4 {
        let mut body = full[..cut].to_vec();
        let before = body.clone();
        match set_int_field(&mut body, 0x8722_5880, 1) {
            Ok(changed) => assert!(!changed && body == before, "cut {cut}"),
            Err(WadError::InvalidProp(_)) => assert_eq!(body, before),
            Err(other) => panic!("cut {cut}: unexpected {other:?}"),
        }
    }
}

#[test]
fn test_a_field_is_found_by_path_through_an_embed() {
    let body = body_with_target_after_every_container();
    let top = field_value(&body, &[0x8722_5880])
        .expect("walk")
        .expect("top");
    assert_eq!(top.as_u32(), Some(2));
    let nested = field_value(&body, &[3, 0x8722_5880])
        .expect("walk")
        .expect("nested");
    assert_eq!((nested.kind, nested.bytes), (1, &[1u8][..]));
    assert_eq!(
        field_value(&body, &[4, 1]).expect("walk"),
        None,
        "a null pointer holds nothing"
    );
    assert_eq!(field_value(&body, &[0x1234]).expect("walk"), None);
    for cut in 0..body.len() {
        assert!(
            !matches!(field_value(&body[..cut], &[0x8722_5880]), Ok(Some(_))),
            "cut {cut} must not yield the last field"
        );
    }
}

#[test]
fn test_every_field_is_flattened_through_every_container() {
    let body = body_with_target_after_every_container();
    let flat = flatten_fields(&body).expect("flatten");
    let paths: Vec<&str> = flat.iter().map(|f| f.path.as_str()).collect();
    assert!(paths.contains(&"00000001"), "string");
    assert!(
        paths.contains(&"00000002.len") && paths.contains(&"00000002[1]"),
        "list and its elements"
    );
    assert!(
        paths.contains(&"00000003.class") && paths.contains(&"00000003/87225880"),
        "embed body"
    );
    assert!(paths.contains(&"00000004.class"), "null pointer");
    assert!(
        paths.contains(&"00000005.some") && paths.contains(&"00000005?"),
        "option"
    );
    assert!(
        paths.contains(&"00000006.len")
            && paths.contains(
                &"00000006{ffffffff}"
                    .replace("ffffffff", "dddddddd")
                    .as_str()
            ),
        "{paths:?}"
    );
    assert_eq!(
        flat.last().map(|f| (f.path.as_str(), f.hex.as_str())),
        Some(("87225880", "02000000"))
    );
}

#[test]
fn test_a_diff_names_only_the_fields_that_changed() {
    let before = body_with_target_after_every_container();
    let mut after = before.clone();
    assert!(set_int_field(&mut after, 0x8722_5880, 1).expect("set"));
    let changes = diff_fields(&before, &after).expect("diff");
    assert_eq!(
        changes,
        vec![FieldChange {
            path: "87225880".into(),
            before: Some("02000000".into()),
            after: Some("01000000".into()),
        }]
    );
    assert!(diff_fields(&before, &before).expect("same").is_empty());
}

#[test]
fn test_flattening_a_hostile_body_is_an_error_not_a_panic() {
    let body = body_with_target_after_every_container();
    for cut in 0..body.len() {
        let _ = flatten_fields(&body[..cut]); // ignore-ok: only the absence of a panic is under test
    }
    let mut deep = 1u16.to_le_bytes().to_vec();
    deep.extend_from_slice(&1u32.to_le_bytes());
    deep.push(FIELD_OPTION);
    for _ in 0..80 {
        deep.extend_from_slice(&[FIELD_OPTION, 1]);
    }
    deep.extend_from_slice(&[FIELD_U32, 1, 7, 0, 0, 0]);
    assert!(flatten_fields(&deep).is_err(), "nesting is bounded");
}

const OLD: u32 = 0x50aa_299e;
const NEW: u32 = 0x51aa_2b31;

fn body_with_references_in_every_container() -> Vec<u8> {
    let mut body = 7u16.to_le_bytes().to_vec();
    field(&mut body, 1, FIELD_HASH, &OLD.to_le_bytes());
    field(&mut body, 2, FIELD_U32, &OLD.to_le_bytes());
    let mut list = vec![FIELD_LINK];
    list.extend_from_slice(&12u32.to_le_bytes());
    list.extend_from_slice(&2u32.to_le_bytes());
    list.extend_from_slice(&0x1234_5678u32.to_le_bytes());
    list.extend_from_slice(&OLD.to_le_bytes());
    field(&mut body, 3, FIELD_LIST, &list);
    let mut embed = 0xAAAA_AAAAu32.to_le_bytes().to_vec();
    embed.extend_from_slice(&11u32.to_le_bytes());
    embed.extend_from_slice(&1u16.to_le_bytes());
    embed.extend_from_slice(&9u32.to_le_bytes());
    embed.push(FIELD_LINK);
    embed.extend_from_slice(&OLD.to_le_bytes());
    field(&mut body, 4, FIELD_EMBED, &embed);
    let mut option = vec![FIELD_HASH, 1];
    option.extend_from_slice(&OLD.to_le_bytes());
    field(&mut body, 5, FIELD_OPTION, &option);
    let mut map = vec![FIELD_LINK, FIELD_LINK];
    map.extend_from_slice(&12u32.to_le_bytes());
    map.extend_from_slice(&1u32.to_le_bytes());
    map.extend_from_slice(&OLD.to_le_bytes());
    map.extend_from_slice(&OLD.to_le_bytes());
    field(&mut body, 6, FIELD_MAP, &map);
    field(&mut body, 7, FIELD_STRING, &[2, 0, b'o', b'k']);
    body
}

#[test]
fn test_every_reference_to_a_moved_key_follows_it_and_nothing_else_moves() {
    let mut body = body_with_references_in_every_container();
    let original = body.clone();
    let map = std::collections::BTreeMap::from([(OLD, NEW)]);
    assert_eq!(remap_references(&mut body, &map).expect("remap"), 6);
    assert_eq!(body.len(), original.len());
    let values = reference_values(&body).expect("references");
    assert!(
        !values.contains(&OLD),
        "no reference is left on the old key"
    );
    assert_eq!(values.iter().filter(|v| **v == NEW).count(), 6);
    assert!(
        values.contains(&0x1234_5678),
        "unrelated references are kept"
    );
    let plain = field_value(&body, &[2]).expect("walk").expect("u32");
    assert_eq!(
        plain.as_u32(),
        Some(OLD),
        "a plain u32 is data, not a reference"
    );
    assert_eq!(
        remap_references(&mut body, &map).expect("again"),
        0,
        "a second pass finds nothing"
    );
}

#[test]
fn test_remapping_a_hostile_body_is_an_error_that_changes_nothing() {
    let full = body_with_references_in_every_container();
    let map = std::collections::BTreeMap::from([(OLD, NEW)]);
    for cut in 0..full.len() {
        let mut body = full[..cut].to_vec();
        let before = body.clone();
        if remap_references(&mut body, &map).is_err() {
            assert_eq!(body, before, "cut {cut}");
        }
    }
    let mut deep = 1u16.to_le_bytes().to_vec();
    deep.extend_from_slice(&1u32.to_le_bytes());
    deep.push(FIELD_OPTION);
    for _ in 0..80 {
        deep.extend_from_slice(&[FIELD_OPTION, 1]);
    }
    deep.extend_from_slice(&[FIELD_LINK, 1]);
    deep.extend_from_slice(&OLD.to_le_bytes());
    assert!(reference_values(&deep).is_err(), "nesting is bounded");
}

#[test]
fn test_the_int_setter_sets_signed_and_unsigned_fields() {
    let mut body = 2u16.to_le_bytes().to_vec();
    field(&mut body, 1, FIELD_I32, &5i32.to_le_bytes());
    field(&mut body, 2, FIELD_U32, &7u32.to_le_bytes());
    assert!(set_int_field(&mut body, 1, 0).expect("set i32"));
    assert!(set_int_field(&mut body, 2, 9).expect("set u32"));
    assert_eq!(
        field_value(&body, &[1])
            .expect("walk")
            .expect("i32")
            .as_u32(),
        Some(0)
    );
    assert_eq!(
        field_value(&body, &[2])
            .expect("walk")
            .expect("u32")
            .as_u32(),
        Some(9)
    );
}

#[test]
fn test_an_unknown_field_type_is_refused() {
    let mut body = 1u16.to_le_bytes().to_vec();
    field(&mut body, 1, 0x7F, &[0; 4]);
    assert!(set_int_field(&mut body, 0x8722_5880, 1).is_err());
}
