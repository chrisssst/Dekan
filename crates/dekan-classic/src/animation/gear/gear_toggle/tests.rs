use dekan_wad::prop::{PropEntry, PropFile};

use super::*;
use crate::vfx_markers::FIELD_STRING;

fn visible(parts: &[&str]) -> Vec<Value> {
    parts
        .iter()
        .map(|p| part_visible(FIELD_POINTER, h(p)))
        .collect()
}

#[test]
fn test_gear_swap_reads_parts_and_equip_animation() {
    let mut equip = 12u16.to_le_bytes().to_vec();
    equip.extend_from_slice(b"Toggle_Swirl");
    let data = Value::Struct {
        kind: 0x83,
        class: h("GearData"),
        fields: vec![
            named("mCharacterSubmeshesToShow", hash_list(&[h("Swirl")])),
            named("mCharacterSubmeshesToHide", hash_list(&[h("Plain")])),
            named(
                "mEquipAnimation",
                Value::Raw {
                    kind: FIELD_STRING,
                    bytes: equip,
                },
            ),
        ],
    };
    let body = tree::write_fields(&[named("mGearData", data)]).expect("gear");
    assert_eq!(
        gear_swap(&body).expect("gear swap"),
        GearSwap {
            show: vec![h("Swirl")],
            hide: vec![h("Plain")],
            equip: Some("Toggle_Swirl".into()),
            transition: None,
        }
    );
}

fn gear_driver(index: u8) -> Value {
    pointer(
        "HasGearDynamicMaterialBoolDriver",
        vec![named(
            "mGearIndex",
            Value::Raw {
                kind: 3,
                bytes: vec![index],
            },
        )],
    )
}

fn material_bin(drivers: Vec<Value>) -> Vec<u8> {
    let condition = pointer(
        "OneTrueMaterialDriver",
        vec![named(
            "mDrivers",
            Value::List {
                kind: FIELD_LIST,
                element: FIELD_POINTER,
                items: drivers,
            },
        )],
    );
    let body = tree::write_fields(&[named("mCondition", condition)]).expect("material");
    serialize_prop_file(&PropFile {
        version: 3,
        links: vec!["DATA/Characters/Viego/Viego.bin".into()],
        entries: vec![
            PropEntry {
                class_hash: h("StaticMaterialDef"),
                key_hash: 1,
                body,
            },
            PropEntry {
                class_hash: h("Other"),
                key_hash: 2,
                body: tree::write_fields(&[named("mName", hash_value(7))]).expect("other"),
            },
        ],
    })
    .expect("bin")
}

#[test]
fn test_gear_drivers_follow_the_visible_form_part() {
    let drivers = visible(&["Base_Sword", "Fighter_Sword", "Tank_Sword"]);
    let (out, count) = drive_by_parts(
        &material_bin(vec![gear_driver(0), gear_driver(2)]),
        &drivers,
    )
    .expect("redrive")
    .expect("drivers found");
    assert_eq!(count, 2);
    let file = parse_prop_file(&out).expect("parse");
    let fields = tree::parse_fields(&file.entries[0].body).expect("fields");
    let drivers = tree::field(&fields, h("mCondition"))
        .and_then(Value::fields)
        .and_then(|f| tree::field(f, h("mDrivers")))
        .and_then(Value::items)
        .expect("drivers");
    let parts: Vec<Vec<u32>> = drivers
        .iter()
        .map(|d| {
            assert_eq!(d.class(), Some(h("SubmeshVisibilityBoolDriver")));
            hashes_in(d.fields().expect("driver fields"), "Submeshes")
        })
        .collect();
    assert_eq!(parts, vec![vec![h("Base_Sword")], vec![h("Tank_Sword")]]);
    let original = parse_prop_file(&material_bin(vec![gear_driver(0)])).expect("parse");
    assert_eq!(
        file.entries[1], original.entries[1],
        "objects without drivers stay byte for byte"
    );
    assert_eq!(file.links, original.links);
}

#[test]
fn test_a_bin_without_gear_drivers_is_not_rewritten() {
    let bin = material_bin(Vec::new());
    assert_eq!(
        drive_by_parts(&bin, &visible(&["A", "B"])).expect("redrive"),
        None
    );
}

#[test]
fn test_drivers_naming_forms_the_skin_lacks_leave_the_bin_untouched() {
    let bin = material_bin(vec![gear_driver(0), gear_driver(5)]);
    assert_eq!(
        drive_by_parts(&bin, &visible(&["A", "B"])).expect("redrive"),
        None
    );
}

#[test]
fn test_a_driver_without_index_is_the_first_form() {
    let missing = pointer("HasGearDynamicMaterialBoolDriver", Vec::new());
    let (out, count) = drive_by_parts(
        &material_bin(vec![missing]),
        &visible(&["Base_Sword", "Other"]),
    )
    .expect("redrive")
    .expect("driver found");
    assert_eq!(count, 1);
    let file = parse_prop_file(&out).expect("parse");
    let fields = tree::parse_fields(&file.entries[0].body).expect("fields");
    let driver = &tree::field(&fields, h("mCondition"))
        .and_then(Value::fields)
        .and_then(|f| tree::field(f, h("mDrivers")))
        .and_then(Value::items)
        .expect("drivers")[0];
    assert_eq!(
        hashes_in(driver.fields().expect("fields"), "Submeshes"),
        vec![h("Base_Sword")]
    );
}
