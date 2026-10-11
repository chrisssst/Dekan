use super::*;
use crate::gear_toggle::named;

fn skn(parts: &[&str], vertex_size: usize) -> Vec<u8> {
    let vertices = 4usize;
    let mut out = Vec::new();
    out.extend_from_slice(&SKN_MAGIC.to_le_bytes());
    out.extend_from_slice(&SKN_MAJOR.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&(parts.len() as u32).to_le_bytes());
    for name in parts {
        let mut raw = [0u8; PART_NAME];
        raw[..name.len()].copy_from_slice(name.as_bytes());
        out.extend_from_slice(&raw);
        for value in [0u32, 4, 0, 6] {
            out.extend_from_slice(&value.to_le_bytes());
        }
    }
    for value in [0u32, 6, vertices as u32, vertex_size as u32, 2] {
        out.extend_from_slice(&value.to_le_bytes());
    }
    out.extend_from_slice(&[7; BOUNDS_SIZE]);
    for index in [0u16, 1, 2, 2, 3, 0] {
        out.extend_from_slice(&index.to_le_bytes());
    }
    out.extend(std::iter::repeat_n(9u8, vertices * vertex_size));
    out.extend_from_slice(&[0; 12]);
    out
}

fn part_names(bytes: &[u8]) -> Vec<String> {
    let parts = count(bytes, 8).expect("parts");
    (0..parts)
        .map(|i| {
            let raw = &bytes[PARTS_AT + i * PART_SIZE..PARTS_AT + i * PART_SIZE + PART_NAME];
            let end = raw.iter().position(|b| *b == 0).unwrap_or(PART_NAME);
            String::from_utf8_lossy(&raw[..end]).into_owned()
        })
        .collect()
}

#[test]
fn test_markers_are_invisible_parts_appended_after_the_mesh() {
    let names = vec!["DekanForm1".to_owned(), "DekanForm2".to_owned()];
    let original = skn(&["Body", "Base_Sword"], 72);
    let out = add_parts(&original, &names, &[]).expect("markers");

    assert_eq!(
        part_names(&out),
        ["Body", "Base_Sword", "DekanForm1", "DekanForm2"]
    );
    let table_end = PARTS_AT + 4 * PART_SIZE;
    assert_eq!(count(&out, table_end + 4), Some(12), "six indices more");
    assert_eq!(count(&out, table_end + 8), Some(10), "six vertices more");
    let second = PARTS_AT + 3 * PART_SIZE + PART_NAME;
    assert_eq!(
        [0, 4, 8, 12].map(|at| count(&out, second + at)),
        [Some(7), Some(3), Some(9), Some(3)]
    );
    let data = table_end + 20 + BOUNDS_SIZE;
    assert_eq!(
        &out[data + 12..data + 24],
        &[4, 0, 5, 0, 6, 0, 7, 0, 8, 0, 9, 0]
    );
    let vertices = data + 24;
    assert_eq!(
        &out[vertices..vertices + 4 * 72],
        &original[PARTS_AT + 2 * PART_SIZE + 60 + 12..PARTS_AT + 2 * PART_SIZE + 60 + 12 + 4 * 72]
    );
    let marker = &out[vertices + 4 * 72..vertices + 5 * 72];
    assert_eq!(
        &marker[..12],
        &[0; 12],
        "every corner at the origin: no area to draw"
    );
    assert_eq!(&marker[16..20], &1f32.to_le_bytes());
    assert_eq!(out.len(), original.len() + 2 * PART_SIZE + 6 * 2 + 6 * 72);
    assert_eq!(&out[out.len() - 12..], &[0; 12]);
}

#[test]
fn test_a_mesh_that_cannot_take_markers_is_left_alone() {
    let names = vec!["DekanForm1".to_owned()];
    assert!(add_parts(&skn(&["Body"], 72), &[], &[]).is_none());
    assert!(add_parts(&skn(&["Body"], 40), &names, &[]).is_none());
    let full: Vec<String> = (0..MAX_PARTS).map(|i| format!("P{i}")).collect();
    let refs: Vec<&str> = full.iter().map(String::as_str).collect();
    assert!(add_parts(&skn(&refs, 52), &names, &[]).is_none());
    assert!(add_parts(&skn(&["Body"], 52), &["x".repeat(PART_NAME)], &[]).is_none());
    let mut old = skn(&["Body"], 52);
    old[4] = 2;
    assert!(add_parts(&old, &names, &[]).is_none());
    let whole = skn(&["Body"], 52);
    assert!(add_parts(&whole[..whole.len() - 40], &names, &[]).is_none());
    assert!(add_parts(&[0x33, 0x22], &names, &[]).is_none());
    assert!(add_parts(&skn(&["Body"], 56), &names, &[]).is_some());
}

fn skin_bin(mesh: Vec<Field>) -> Vec<u8> {
    use dekan_wad::prop::{PropEntry, PropFile};
    let mesh = Value::Struct {
        kind: 0x83,
        class: h("SkinMeshDataProperties"),
        fields: mesh,
    };
    serialize_prop_file(&PropFile {
        version: 3,
        links: Vec::new(),
        entries: vec![PropEntry {
            class_hash: h(SKIN_CLASS),
            key_hash: 1,
            body: tree::write_fields(&[named("skinMeshProperties", mesh)]).expect("skin"),
        }],
    })
    .expect("bin")
}

fn text_field(name: &str, value: &str) -> Field {
    named(name, string_value(value).expect("string"))
}

#[test]
fn test_markers_start_hidden_after_the_parts_the_skin_already_hides() {
    let bin = skin_bin(vec![
        text_field("simpleSkin", "ASSETS/Viego_Skin43.skn"),
        text_field("initialSubmeshToHide", "Mage_Sword Horns"),
    ]);
    assert_eq!(
        mesh_text(&bin, "simpleSkin").expect("path"),
        Some("ASSETS/Viego_Skin43.skn".into())
    );
    let names = marker_names(3);
    let out = hide_at_start(&bin, &names).expect("hide").expect("changed");
    let file = parse_prop_file(&out).expect("prop");
    let mut fields = tree::parse_fields(&file.entries[0].body).expect("fields");
    let hidden = mesh_fields(&mut fields)
        .and_then(|mesh| tree::field(mesh, h("initialSubmeshToHide")))
        .and_then(text);
    assert_eq!(
        hidden.as_deref(),
        Some("Mage_Sword Horns DekanForm1 DekanForm2")
    );

    let bare = skin_bin(Vec::new());
    assert_eq!(mesh_text(&bare, "simpleSkin").expect("path"), None);
    let out = hide_at_start(&bare, &names)
        .expect("hide")
        .expect("changed");
    let file = parse_prop_file(&out).expect("prop");
    let mut fields = tree::parse_fields(&file.entries[0].body).expect("fields");
    assert_eq!(
        mesh_fields(&mut fields)
            .and_then(|mesh| tree::field(mesh, h("initialSubmeshToHide")))
            .and_then(text)
            .as_deref(),
        Some("DekanForm1 DekanForm2")
    );
    let no_skin = serialize_prop_file(&dekan_wad::prop::PropFile {
        version: 3,
        links: Vec::new(),
        entries: Vec::new(),
    })
    .expect("bin");
    assert_eq!(mesh_text(&no_skin, "simpleSkin").expect("path"), None);
    assert_eq!(hide_at_start(&no_skin, &names).expect("hide"), None);
}

#[test]
fn test_a_part_copy_reuses_the_vertices_and_repeats_the_indices() {
    let names = vec!["DekanForm1".to_owned()];
    let original = skn(&["Body", "Hair"], 52);
    let out = add_parts(&original, &names, &[("hair".into(), "Hair_F1".into())]).expect("parts");
    let table_end = PARTS_AT + 4 * PART_SIZE;
    let copy = PARTS_AT + 2 * PART_SIZE;
    assert_eq!(part_name(&out[copy..copy + PART_NAME]), "hair_f1");
    assert_eq!(
        [0, 4, 8, 12].map(|at| count(&out, copy + PART_NAME + at)),
        [Some(0), Some(4), Some(6), Some(6)]
    );
    let marker = PARTS_AT + 3 * PART_SIZE + PART_NAME;
    assert_eq!(
        [0, 8].map(|at| count(&out, marker + at)),
        [Some(4), Some(12)],
        "markers follow the copied indices"
    );
    assert_eq!(count(&out, table_end + 4), Some(15));
    let data = table_end + 20 + BOUNDS_SIZE;
    assert_eq!(&out[data + 12..data + 24], &out[data..data + 12]);
    assert!(add_parts(&original, &names, &[("missing".into(), "X_F1".into())]).is_none());
}
