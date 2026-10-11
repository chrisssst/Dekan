use super::*;
use dekan_wad::prop::{PropEntry, PropFile};

const FIELD_FILE: u8 = 18;
const FIELD_FLOAT: u8 = 10;

fn file(hash: u64) -> Value {
    Value::Raw {
        kind: FIELD_FILE,
        bytes: hash.to_le_bytes().to_vec(),
    }
}

fn float(value: f32) -> Value {
    Value::Raw {
        kind: FIELD_FLOAT,
        bytes: value.to_le_bytes().to_vec(),
    }
}

fn gear(mesh: &str, skeleton: &str, texture: u64, scale: f32) -> Vec<u8> {
    let properties = embed(
        "SkinMeshDataProperties",
        vec![
            named("skeleton", text(skeleton)),
            named("simpleSkin", text(mesh)),
            named("texture", file(texture)),
            named("skinScale", float(scale)),
            named("selfIllumination", float(0.7)),
        ],
    );
    let data = embed(
        "GearData",
        vec![
            named("skinMeshProperties", properties),
            named("mEquipAnimation", text("Equip")),
        ],
    );
    tree::write_fields(&[named("mGearData", data)]).expect("gear")
}

fn parts() -> Vec<Vec<String>> {
    vec![
        vec!["Light_Body".into(), "Light_Staff".into()],
        vec!["Air_Body".into(), "Air_Collar".into()],
    ]
}

fn data_of(body: &[u8]) -> Vec<Field> {
    let fields = tree::parse_fields(body).expect("fields");
    tree::field(&fields, h("mGearData"))
        .and_then(Value::fields)
        .expect("data")
        .to_vec()
}

fn hashes(data: &[Field], name: &str) -> Vec<u32> {
    tree::field(data, h(name))
        .and_then(Value::items)
        .map(|i| i.iter().filter_map(Value::as_u32).collect())
        .unwrap_or_default()
}

#[test]
fn each_gear_names_its_own_model_and_scale() {
    let models = gear_models(&[
        gear("A.skn", "A.skl", 1, 0.95),
        gear("B.skn", "B.skl", 2, 0.95),
    ])
    .expect("models");
    assert_eq!(models[1].mesh.as_deref(), Some("B.skn"));
    assert_eq!(models[1].skeleton.as_deref(), Some("B.skl"));
    assert_eq!(models[0].scale, models[1].scale);
    let plain =
        tree::write_fields(&[named("mGearData", embed("GearData", Vec::new()))]).expect("plain");
    assert_eq!(
        gear_models(&[plain]).expect("plain")[0],
        GearModel {
            mesh: None,
            skeleton: None,
            scale: None
        }
    );
}

#[test]
fn a_gear_with_its_own_model_becomes_a_gear_that_shows_its_parts_with_its_own_texture() {
    let bodies = [
        gear("A.skn", "A.skl", 1, 0.95),
        gear("B.skn", "B.skl", 0xA1, 0.95),
    ];
    let normalized = as_part_gears(&bodies, &parts()).expect("normalize");
    let air = data_of(&normalized[1]);
    assert_eq!(
        hashes(&air, "mCharacterSubmeshesToShow"),
        [h("Air_Body"), h("Air_Collar")]
    );
    assert_eq!(
        hashes(&air, "mCharacterSubmeshesToHide"),
        [h("Light_Body"), h("Light_Staff")]
    );
    let mesh = tree::field(&air, h("skinMeshProperties"))
        .and_then(Value::fields)
        .expect("mesh");
    assert!(
        tree::field(mesh, h("simpleSkin")).is_none(),
        "the model swap is gone"
    );
    assert!(tree::field(mesh, h("skeleton")).is_none());
    assert_eq!(tree::field(mesh, h("skinScale")), Some(&float(0.95)));
    let overrides = tree::field(mesh, h("materialOverride"))
        .and_then(Value::items)
        .expect("overrides");
    assert_eq!(overrides.len(), 2);
    for (entry, part) in overrides.iter().zip(["Air_Body", "Air_Collar"]) {
        let fields = entry.fields().expect("entry");
        assert_eq!(tree::field(fields, h("submesh")), Some(&text(part)));
        assert_eq!(tree::field(fields, h("texture")), Some(&file(0xA1)));
    }
    assert_eq!(
        tree::field(&air, h("mEquipAnimation")),
        Some(&text("Equip")),
        "everything else of the gear stays"
    );
    assert!(as_part_gears(&bodies, &parts()[..1]).is_err());
}

#[test]
fn clips_that_hide_parts_are_found_anywhere_in_the_graph() {
    let event = crate::gear_toggle::visibility_event(&[h("Air_Body")], &[h("Air_Collar")]);
    let body = tree::write_fields(&[named(
        "mClipDataMap",
        Value::List {
            kind: FIELD_LIST,
            element: 0x82,
            items: vec![event],
        },
    )])
    .expect("body");
    let bin = serialize_prop_file(&PropFile {
        version: 3,
        links: Vec::new(),
        entries: vec![PropEntry {
            class_hash: h("AnimationGraphData"),
            key_hash: 1,
            body,
        }],
    })
    .expect("bin");
    let hidden = hidden_by_clips(&bin).expect("hidden");
    assert_eq!(hidden, BTreeSet::from([h("Air_Collar")]));
}

#[test]
fn each_extra_form_is_marked_by_its_largest_part_that_no_clip_hides() {
    let sizes = |part: &str| match part {
        "Air_Body" => 900,
        "Air_Collar" => 50,
        _ => 10,
    };
    assert_eq!(
        marker_parts(&parts(), sizes, &BTreeSet::new()).expect("markers"),
        [h("Air_Body")]
    );
    assert_eq!(
        marker_parts(&parts(), sizes, &BTreeSet::from([h("Air_Body")])).expect("markers"),
        [h("Air_Collar")]
    );
    let all = BTreeSet::from([h("Air_Body"), h("Air_Collar")]);
    assert!(marker_parts(&parts(), sizes, &all).is_err());
}

#[test]
fn the_skin_points_at_the_merged_model_under_new_paths() {
    assert_eq!(
        merged_path("ASSETS/Characters/Lux/Skins/Skin07/Lux_Skin07.skn"),
        "ASSETS/Characters/Lux/Skins/Skin07/Lux_Skin07_DekanForms.skn"
    );
    let skin = tree::write_fields(&[named(
        "skinMeshProperties",
        embed(
            "SkinMeshDataProperties",
            vec![
                named("simpleSkin", text("old.skn")),
                named("skeleton", text("old.skl")),
            ],
        ),
    )])
    .expect("skin");
    let bin = serialize_prop_file(&PropFile {
        version: 3,
        links: Vec::new(),
        entries: vec![PropEntry {
            class_hash: h(SKIN_CLASS),
            key_hash: 7,
            body: skin,
        }],
    })
    .expect("bin");
    let pointed = point_at(&bin, "new.skn", "new.skl").expect("point");
    assert_eq!(
        crate::form_marker::mesh_text(&pointed, "simpleSkin")
            .expect("mesh")
            .as_deref(),
        Some("new.skn")
    );
    assert_eq!(
        crate::form_marker::mesh_text(&pointed, "skeleton")
            .expect("skeleton")
            .as_deref(),
        Some("new.skl")
    );
}
