use dekan_wad::prop::tree::Field;

use super::*;
use crate::gear_toggle::{hash_value, named, pointer};

fn pair(clip: &str, value: Option<f32>) -> Value {
    let mut fields = vec![named("mClipName", hash_value(h(clip)))];
    if let Some(value) = value {
        fields.push(named(
            "mValue",
            Value::Raw {
                kind: 10,
                bytes: value.to_le_bytes().to_vec(),
            },
        ));
    }
    Value::Struct {
        kind: 0x83,
        class: h("ConditionFloatPairData"),
        fields,
    }
}

fn by_gear(updater: &str, pairs: Vec<Value>) -> Value {
    pointer(
        "ConditionFloatClipData",
        vec![
            named("Updater", pointer(updater, Vec::new())),
            named(
                "mConditionFloatPairDataList",
                Value::List {
                    kind: 0x80,
                    element: 0x83,
                    items: pairs,
                },
            ),
        ],
    )
}

fn equip() -> Value {
    by_gear(
        "EquippedGearParametricUpdater",
        vec![
            pair("Toggle_Base", None),
            pair("Toggle_Fighter", Some(1.0)),
            pair("Toggle_Tank", Some(2.0)),
        ],
    )
}

#[test]
fn test_a_gear_choice_takes_the_last_entry_not_above_the_form() {
    assert_eq!(gear_choice(&equip(), 0), Some(h("Toggle_Base")));
    assert_eq!(gear_choice(&equip(), 1), Some(h("Toggle_Fighter")));
    assert_eq!(gear_choice(&equip(), 2), Some(h("Toggle_Tank")));
    assert_eq!(
        gear_choice(&equip(), 5),
        Some(h("Toggle_Tank")),
        "a form past the last entry keeps the last one"
    );
    let speed = by_gear("MoveSpeedParametricUpdater", vec![pair("Run", None)]);
    assert_eq!(
        gear_choice(&speed, 1),
        None,
        "only the equipped gear is fixed per form"
    );
    assert_eq!(gear_choice(&pointer("AtomicClipData", Vec::new()), 1), None);
}

#[test]
fn test_each_form_copy_plays_the_variant_of_its_gear() {
    let copy = |form: usize, key: u32| h(&format!("F{form}_{key:08x}"));
    let originals: BTreeMap<u32, Value> = [
        (h("Equip"), equip()),
        (h("Toggle_Base"), pointer("AtomicClipData", Vec::new())),
        (
            h("Toggle_Fighter"),
            pointer("AtomicClipData", vec![named("mName", hash_value(1))]),
        ),
        (
            h("Toggle_Tank"),
            pointer("AtomicClipData", vec![named("mName", hash_value(2))]),
        ),
    ]
    .into_iter()
    .collect();
    let mut entries: Vec<(Value, Value)> = (0..2)
        .flat_map(|form| {
            originals
                .iter()
                .map(move |(k, v)| (hash_value(copy(form, *k)), v.clone()))
        })
        .collect();
    specialize_gear_choices(&mut entries, &originals, 2, copy);
    let get = |key: u32| {
        entries
            .iter()
            .find(|(k, _)| k.as_u32() == Some(key))
            .map(|(_, v)| v.clone())
            .expect("entry")
    };
    assert_eq!(get(copy(1, h("Equip"))), originals[&h("Toggle_Fighter")]);
    assert_eq!(get(copy(0, h("Equip"))), originals[&h("Toggle_Base")]);
    let fields: Vec<Field> = get(copy(1, h("Toggle_Tank")))
        .fields()
        .expect("tank")
        .to_vec();
    assert_eq!(
        fields,
        originals[&h("Toggle_Tank")]
            .fields()
            .expect("tank")
            .to_vec()
    );
}
