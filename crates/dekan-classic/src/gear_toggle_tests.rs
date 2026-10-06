use dekan_wad::prop::{PropEntry, PropFile};

use super::*;

const GRAPH_KEY: u32 = 0x00AB_CDEF;

fn atomic(track: &str) -> Value {
    pointer(
        "AtomicClipData",
        vec![named("mTrackDataName", hash_value(h(track)))],
    )
}

fn condition(moving: &str, still: &str) -> Value {
    pointer(
        "ConditionBoolClipData",
        vec![
            named("mTrueConditionClipName", hash_value(h(moving))),
            named("mFalseConditionClipName", hash_value(h(still))),
        ],
    )
}

fn graph_bin(clips: Vec<(&str, Value)>) -> Vec<u8> {
    let map = Value::Map {
        key: FIELD_HASH,
        value: FIELD_POINTER,
        entries: clips
            .into_iter()
            .map(|(name, clip)| (hash_value(h(name)), clip))
            .collect(),
    };
    let body = tree::write_fields(&[named("mClipDataMap", map)]).expect("graph fields");
    serialize_prop_file(&PropFile {
        version: 3,
        links: Vec::new(),
        entries: vec![PropEntry {
            class_hash: h("AnimationGraphData"),
            key_hash: GRAPH_KEY,
            body,
        }],
    })
    .expect("graph bin")
}

fn clips_of(bin: &[u8]) -> Vec<(u32, Value)> {
    let file = parse_prop_file(bin).expect("parse");
    let graph = tree::parse_fields(&file.entries[0].body).expect("fields");
    match tree::field(&graph, h("mClipDataMap")) {
        Some(Value::Map { entries, .. }) => entries
            .iter()
            .map(|(k, v)| (k.as_u32().expect("hash key"), v.clone()))
            .collect(),
        _ => panic!("no clip map"),
    }
}

fn clip(clips: &[(u32, Value)], key: u32) -> &Value {
    &clips
        .iter()
        .find(|(k, _)| *k == key)
        .unwrap_or_else(|| panic!("clip {key:08x} missing"))
        .1
}

fn swap(show: &[&str], equip: Option<&str>) -> GearSwap {
    GearSwap {
        show: show.iter().map(|s| h(s)).collect(),
        hide: Vec::new(),
        equip: equip.map(str::to_owned),
    }
}

fn three_swords() -> Vec<GearSwap> {
    vec![
        swap(&["Base_Sword"], Some("Toggle_Base")),
        swap(&["Fighter_Sword"], Some("Toggle_Fighter")),
        swap(&["Tank_Sword"], Some("Toggle_Tank")),
    ]
}

fn sword_graph() -> Vec<u8> {
    graph_bin(vec![
        ("Idle1", atomic("Default")),
        ("Toggle_Base", condition("Base_Run", "Base_Idle")),
        ("Base_Run", atomic("Default")),
        ("Base_Idle", atomic("Default")),
        ("Toggle_Fighter", condition("Fighter_Run", "Fighter_Idle")),
        ("Fighter_Run", atomic("Default")),
        ("Fighter_Idle", atomic("Default")),
        ("Toggle_Tank", condition("Tank_Run", "Tank_Idle")),
        ("Tank_Run", atomic("Default")),
        ("Tank_Idle", atomic("Default")),
    ])
}

fn visibility(clip: &Value) -> (Vec<u32>, Vec<u32>) {
    let events = tree::field(clip.fields().expect("clip fields"), h("mEventDataMap"));
    let Some(Value::Map { entries, .. }) = events else {
        panic!("carrier without events");
    };
    let event = &entries
        .iter()
        .find(|(k, _)| k.as_u32() == Some(h("DekanGearSwap")))
        .expect("swap event")
        .1;
    let fields = event.fields().expect("event fields");
    (
        hashes_in(fields, "mShowSubmeshList"),
        hashes_in(fields, "mHideSubmeshList"),
    )
}

fn branches(clip: &Value) -> (u32, u32) {
    let fields = clip.fields().expect("condition fields");
    (
        tree::field(fields, h("mTrueConditionClipName"))
            .and_then(Value::as_u32)
            .expect("true branch"),
        tree::field(fields, h("mFalseConditionClipName"))
            .and_then(Value::as_u32)
            .expect("false branch"),
    )
}

fn carrier(form: usize, start: &str) -> u32 {
    h(&format!("DekanGear{form}_{:08x}", h(start)))
}

fn toggled() -> Vec<(u32, Value)> {
    clips_of(
        &add_toggle(&sword_graph(), GRAPH_KEY, &three_swords())
            .expect("toggle")
            .expect("graph changed"),
    )
}

#[test]
fn test_each_form_needs_a_part_only_it_shows() {
    assert_eq!(
        markers(&three_swords()),
        Some(vec![h("Base_Sword"), h("Fighter_Sword"), h("Tank_Sword")])
    );
    let shared = vec![swap(&["Sword"], None), swap(&["Sword", "Glow"], None)];
    assert_eq!(
        markers(&shared),
        None,
        "the first form has no part of its own"
    );
    assert_eq!(
        markers(&three_swords()[..1]),
        None,
        "one form has nothing to cycle"
    );
}

#[test]
fn test_toggle_cycles_through_the_forms_by_the_visible_part() {
    let clips = toggled();
    let (visible, otherwise) = branches(clip(&clips, h("Toggle")));
    assert_eq!(
        visible,
        carrier(1, "Toggle_Fighter"),
        "base sword shown: go to the second"
    );
    assert_eq!(otherwise, h("DekanToggle1"));
    let (visible, otherwise) = branches(clip(&clips, h("DekanToggle1")));
    assert_eq!(visible, carrier(2, "Toggle_Tank"));
    assert_eq!(otherwise, h("DekanToggle2"));
    let (visible, otherwise) = branches(clip(&clips, h("DekanToggle2")));
    assert_eq!(
        visible,
        carrier(0, "Toggle_Base"),
        "the last form goes back to the first"
    );
    assert_eq!(otherwise, carrier(1, "Toggle_Fighter"));
}

#[test]
fn test_the_equip_animation_is_copied_with_the_parts_to_show_and_hide() {
    let clips = toggled();
    let (moving, still) = branches(clip(&clips, carrier(1, "Toggle_Fighter")));
    let mut expected_hidden = vec![h("Base_Sword"), h("Tank_Sword")];
    expected_hidden.sort_unstable();
    for atomic_copy in [moving, still] {
        let (show, hide) = visibility(clip(&clips, atomic_copy));
        assert_eq!(show, vec![h("Fighter_Sword")]);
        assert_eq!(hide, expected_hidden);
    }
}

#[test]
fn test_the_game_clips_stay_byte_for_byte() {
    let before = clips_of(&sword_graph());
    let after = toggled();
    assert_eq!(&after[..before.len()], &before[..]);
    assert!(after.len() > before.len());
}

#[test]
fn test_a_form_without_equip_animation_rides_on_the_idle() {
    let swaps = vec![swap(&["Base_Sword"], None), swap(&["Fighter_Sword"], None)];
    let out = add_toggle(&sword_graph(), GRAPH_KEY, &swaps)
        .expect("toggle")
        .expect("graph changed");
    let clips = clips_of(&out);
    let (show, _) = visibility(clip(&clips, carrier(1, "Idle1")));
    assert_eq!(show, vec![h("Fighter_Sword")]);
}

#[test]
fn test_graphs_that_already_cycle_or_cannot_are_left_alone() {
    let with_toggle = graph_bin(vec![
        ("Toggle", atomic("Default")),
        ("Idle1", atomic("Default")),
    ]);
    assert_eq!(
        add_toggle(&with_toggle, GRAPH_KEY, &three_swords()).expect("toggle"),
        None
    );
    let no_carrier = graph_bin(vec![("Run", atomic("Default"))]);
    let swaps = vec![swap(&["A"], None), swap(&["B"], None)];
    assert_eq!(
        add_toggle(&no_carrier, GRAPH_KEY, &swaps).expect("toggle"),
        None
    );
    assert_eq!(
        add_toggle(&sword_graph(), 0x1234, &three_swords()).expect("toggle"),
        None,
        "graph key not in the bin"
    );
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
    let markers = [h("Base_Sword"), h("Fighter_Sword"), h("Tank_Sword")];
    let (out, count) = drive_by_parts(
        &material_bin(vec![gear_driver(0), gear_driver(2)]),
        &markers,
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
    assert_eq!(drive_by_parts(&bin, &[1, 2]).expect("redrive"), None);
}

#[test]
fn test_drivers_naming_forms_the_skin_lacks_leave_the_bin_untouched() {
    let bin = material_bin(vec![gear_driver(0), gear_driver(5)]);
    assert_eq!(drive_by_parts(&bin, &[1, 2]).expect("redrive"), None);
}

#[test]
fn test_a_driver_without_index_is_the_first_form() {
    let missing = pointer("HasGearDynamicMaterialBoolDriver", Vec::new());
    let (out, count) = drive_by_parts(&material_bin(vec![missing]), &[h("Base_Sword"), h("Other")])
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
