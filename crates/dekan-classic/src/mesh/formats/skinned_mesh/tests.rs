use super::*;

pub(crate) fn vertex(position: [f32; 3], influences: [u8; 4], color: Option<[u8; 4]>) -> Vec<u8> {
    let mut v = Vec::new();
    position
        .iter()
        .for_each(|f| v.extend_from_slice(&f.to_le_bytes()));
    v.extend_from_slice(&influences);
    [1.0f32, 0.0, 0.0, 0.0]
        .iter()
        .for_each(|f| v.extend_from_slice(&f.to_le_bytes()));
    [0.0f32, 1.0, 0.0, 0.5, 0.5]
        .iter()
        .for_each(|f| v.extend_from_slice(&f.to_le_bytes()));
    if let Some(color) = color {
        v.extend_from_slice(&color);
    }
    v
}

pub(crate) fn mesh(parts: &[(&str, usize)], color: bool, influence: u8) -> SkinnedMesh {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    let mut submeshes = Vec::new();
    for (i, (name, triangles)) in parts.iter().enumerate() {
        let first_vertex = vertices.len() / if color { COLOR_VERTEX } else { BASIC_VERTEX };
        let first_index = indices.len();
        for t in 0..*triangles {
            for corner in 0..3u16 {
                let n = first_vertex + t * 3 + usize::from(corner);
                let x = f32::from(u16::try_from(n).expect("small")) + i as f32;
                vertices.extend(vertex(
                    [x, -x, 1.0],
                    [influence, 0, 0, 0],
                    color.then_some([10, 20, 30, 40]),
                ));
                indices.push(u16::try_from(n).expect("index"));
            }
        }
        submeshes.push(Submesh {
            name: (*name).to_owned(),
            first_vertex: u32::try_from(first_vertex).expect("v"),
            vertex_count: u32::try_from(triangles * 3).expect("v"),
            first_index: u32::try_from(first_index).expect("i"),
            index_count: u32::try_from(triangles * 3).expect("i"),
        });
    }
    SkinnedMesh {
        minor: 1,
        flags: 0,
        vertex_type: u32::from(color),
        vertex_size: if color { COLOR_VERTEX } else { BASIC_VERTEX },
        bounds: [-1.0, -1.0, -1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.8],
        submeshes,
        indices,
        vertices,
        trailer: vec![0; 12],
    }
}

#[test]
fn a_written_mesh_reads_back_whole_and_rewrites_byte_for_byte() {
    for color in [false, true] {
        let original = mesh(&[("Body", 2), ("Staff", 1)], color, 3);
        let bytes = write(&original).expect("write");
        let parsed = parse(&bytes).expect("parse");
        assert_eq!(parsed, original);
        assert_eq!(write(&parsed).expect("rewrite"), bytes);
        assert_eq!(parsed.vertex_count(), 9);
        assert_eq!(
            parsed.vertex(8).map(<[u8]>::len),
            Some(original.vertex_size)
        );
        assert!(parsed.vertex(9).is_none());
    }
}

#[test]
fn damaged_meshes_are_typed_errors_never_panics() {
    let bytes = write(&mesh(&[("Body", 2)], true, 1)).expect("write");
    for cut in 0..bytes.len() - 12 {
        assert!(parse(&bytes[..cut]).is_err(), "cut at {cut}");
    }
    let mut old_version = bytes.clone();
    old_version[4] = 2;
    assert!(parse(&old_version).is_err());
    let mut wild_index = bytes.clone();
    let indices_at = SUBMESHES_AT + SUBMESH + 60;
    wild_index[indices_at..indices_at + 2].copy_from_slice(&500u16.to_le_bytes());
    assert!(parse(&wild_index).is_err());
    let mut wild_part = bytes;
    let count_at = SUBMESHES_AT + NAME + 4;
    wild_part[count_at..count_at + 4].copy_from_slice(&99u32.to_le_bytes());
    assert!(parse(&wild_part).is_err());
}

#[test]
fn a_name_that_does_not_fit_its_field_is_refused() {
    let mut long = mesh(&[("Body", 1)], false, 0);
    long.submeshes[0].name = "x".repeat(NAME);
    assert!(write(&long).is_err());
}
