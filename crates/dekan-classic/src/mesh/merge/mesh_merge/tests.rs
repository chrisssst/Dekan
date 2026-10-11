use super::*;
use crate::mesh::skeleton::tests::{joint, skeleton};
use crate::mesh::skinned_mesh::tests::mesh;

fn base_form() -> FormModel {
    FormModel {
        skeleton: skeleton(
            vec![
                joint("Root", -1, [0.0; 3]),
                joint("Spine", 0, [0.0, 10.0, 0.0]),
                joint("R_Shoulder", 1, [5.0, 0.0, 0.0]),
            ],
            vec![1, 2],
        ),
        mesh: mesh(&[("Light_Body", 2), ("Light_Staff", 1)], true, 1),
    }
}

fn storm_form() -> FormModel {
    let mut shoulder = joint("R_Shoulder", 1, [5.0, 0.0, 0.0]);
    shoulder.inverse_bind.rotation = [0.0, 0.0, 0.452, 0.892];
    FormModel {
        skeleton: skeleton(
            vec![
                joint("Root", -1, [0.0; 3]),
                joint("Spine", 0, [0.0, 10.0, 0.0]),
                joint("Wing", 1, [0.0, 2.0, -3.0]),
                shoulder,
            ],
            vec![3, 2, 1],
        ),
        mesh: mesh(&[("Storm_Body", 1), ("Storm_Wings", 1)], false, 0),
    }
}

#[test]
fn forms_share_bones_by_name_and_a_different_bind_gets_a_skinning_twin_under_the_shared_bone() {
    let merged = merge_forms(&[base_form(), storm_form()]).expect("merge");
    let names: Vec<&str> = merged
        .skeleton
        .joints
        .iter()
        .map(|j| j.name.as_str())
        .collect();
    assert_eq!(
        names,
        [
            "Root",
            "Spine",
            "R_Shoulder",
            "Wing",
            "DekanForm1_R_Shoulder"
        ]
    );
    assert_eq!(merged.twins, 1);
    let twin = &merged.skeleton.joints[4];
    assert_eq!(twin.parent, 2, "the twin follows the shared shoulder");
    assert_eq!(twin.local, Transform::IDENTITY);
    assert_eq!(twin.inverse_bind.rotation, [0.0, 0.0, 0.452, 0.892]);
    assert_eq!(twin.hash, joint_hash("DekanForm1_R_Shoulder"));
    assert_eq!(
        merged.skeleton.joints[3].parent, 1,
        "the new wing hangs from the shared spine"
    );
    assert_eq!(
        merged.skeleton.influences,
        [1, 2, 4, 3],
        "the base keeps its influences in place; the rest is appended"
    );
}

#[test]
fn vertices_are_remapped_widened_to_the_common_layout_and_parts_keep_their_own_ranges() {
    let merged = merge_forms(&[base_form(), storm_form()]).expect("merge");
    let mesh = &merged.mesh;
    assert_eq!(mesh.vertex_size, COLOR_VERTEX);
    assert_eq!(mesh.vertex_type, 1);
    assert_eq!(mesh.vertex_count(), 9 + 6);
    let base_vertex = mesh.vertex(0).expect("base vertex");
    assert_eq!(
        base_vertex[INFLUENCES_AT], 1,
        "base influences are untouched"
    );
    assert_eq!(&base_vertex[COLOR_AT..], &[10, 20, 30, 40]);
    let storm_vertex = mesh.vertex(9).expect("storm vertex");
    assert_eq!(
        storm_vertex[INFLUENCES_AT], 2,
        "storm influence 0 is its shoulder, which skins through the twin"
    );
    assert_eq!(
        &storm_vertex[COLOR_AT..],
        &WHITE,
        "a mesh without colour draws white"
    );
    assert_eq!(
        merged.parts,
        [
            vec!["Light_Body", "Light_Staff"],
            vec!["Storm_Body", "Storm_Wings"]
        ]
    );
    let storm = &mesh.submeshes[2];
    assert_eq!((storm.first_vertex, storm.first_index), (9, 9));
    assert!(mesh.indices[9..].iter().all(|i| (9..15).contains(i)));
    let written = crate::mesh::skinned_mesh::write(mesh).expect("write");
    assert_eq!(
        &crate::mesh::skinned_mesh::parse(&written).expect("parse"),
        mesh
    );
    let skeleton = crate::mesh::skeleton::write(&merged.skeleton).expect("write");
    assert_eq!(
        crate::mesh::skeleton::parse(&skeleton).expect("parse"),
        merged.skeleton
    );
}

#[test]
fn the_merged_bounds_hold_every_form() {
    let mut far = storm_form();
    far.mesh.bounds = [5.0, 5.0, 5.0, 9.0, 9.0, 9.0, 7.0, 7.0, 7.0, 3.5];
    let merged = merge_forms(&[base_form(), far]).expect("merge");
    let b = merged.mesh.bounds;
    assert_eq!(&b[..6], &[-1.0, -1.0, -1.0, 9.0, 9.0, 9.0]);
    let center = [4.0f32; 3];
    let reach = |c: [f32; 3], r: f32| {
        (0..3)
            .map(|i| (c[i] - center[i]).powi(2))
            .sum::<f32>()
            .sqrt()
            + r
    };
    assert!(b[9] >= reach([0.0; 3], 1.8) && b[9] >= reach([7.0; 3], 3.5));
}

#[test]
fn forms_that_cannot_share_one_model_are_refused_with_the_reason() {
    let mut same_name = storm_form();
    same_name.mesh.submeshes[0].name = "light_body".into();
    let err = merge_forms(&[base_form(), same_name]).expect_err("name clash");
    assert!(err.to_string().contains("Light_Body") || err.to_string().contains("light_body"));

    let mut orphan = storm_form();
    orphan.skeleton.joints[2] = joint("Tail", 3, [0.0, 0.0, -1.0]);
    orphan.skeleton.joints[3] = joint("Fin", 1, [0.0, 1.0, -1.0]);
    let err = merge_forms(&[base_form(), orphan]).expect_err("parent after child");
    assert!(err.to_string().contains("before its parent"), "{err}");

    let mut tangent = storm_form();
    tangent.mesh.vertex_size = crate::mesh::skinned_mesh::TANGENT_VERTEX;
    assert!(merge_forms(&[base_form(), tangent]).is_err());

    let mut wild = storm_form();
    wild.mesh.vertices[INFLUENCES_AT] = 9;
    assert!(merge_forms(&[base_form(), wild]).is_err());

    assert!(merge_forms(&[]).is_err());
}

#[test]
fn too_many_vertices_or_influences_are_refused() {
    let big = |name: &str| FormModel {
        skeleton: skeleton(vec![joint("Root", -1, [0.0; 3])], vec![0]),
        mesh: mesh(&[(name, 11_000)], false, 0),
    };
    assert!(merge_forms(&[big("A"), big("B")]).is_err());

    let mut joints = vec![joint("Root", -1, [0.0; 3])];
    let mut influences = Vec::new();
    for i in 1..=200u16 {
        joints.push(joint(&format!("J{i}"), 0, [f32::from(i), 0.0, 0.0]));
        influences.push(i);
    }
    let crowded = |bind: f32| {
        let mut form = FormModel {
            skeleton: skeleton(joints.clone(), influences.clone()),
            mesh: mesh(&[(&format!("P{bind}"), 1)], false, 0),
        };
        form.skeleton
            .joints
            .iter_mut()
            .for_each(|j| j.inverse_bind.translation[1] = bind);
        form
    };
    let err = merge_forms(&[crowded(0.0), crowded(50.0)]).expect_err("joints");
    assert!(err.to_string().contains("more than"), "{err}");
}
