use dekan_wad::prop::{PropEntry, PropFile};

use super::*;
use crate::gear_toggle::{GearSwap, hash_value};
use crate::vfx_markers::text;

fn idle(key: &str) -> Value {
    embed(
        "SkinCharacterDataProperties_CharacterIdleEffect",
        vec![
            named("effectKey", hash_value(h(key))),
            named("boneName", text("Head")),
            named("mUnknownExtra", hash_value(1)),
        ],
    )
}

fn material(part: &str, link: u32) -> Value {
    embed(
        "SkinMeshDataProperties_MaterialOverride",
        vec![
            named("Material", hash_value(link)),
            named("submesh", text(part)),
        ],
    )
}

fn list(items: Vec<Value>) -> Value {
    Value::List {
        kind: 0x80,
        element: FIELD_EMBED,
        items,
    }
}

fn gear(fields: Vec<Field>) -> Vec<u8> {
    tree::write_fields(&[named("mGearData", embed("GearData", fields))]).expect("gear")
}

fn mesh(fields: Vec<Field>) -> Field {
    named(
        "skinMeshProperties",
        embed("SkinMeshDataProperties", fields),
    )
}

fn skin(fields: Vec<Field>) -> Vec<u8> {
    serialize_prop_file(&PropFile {
        version: 3,
        links: Vec::new(),
        entries: vec![PropEntry {
            class_hash: h(SKIN_CLASS),
            key_hash: 1,
            body: tree::write_fields(&fields).expect("skin"),
        }],
    })
    .expect("bin")
}

fn enabled() -> Field {
    named(
        "EnableOverrideIdleEffects",
        Value::Raw {
            kind: 1,
            bytes: vec![1],
        },
    )
}

fn diana() -> Vec<Vec<u8>> {
    vec![
        gear(vec![
            enabled(),
            named("OverrideIdleEffects", list(vec![idle("Moon")])),
        ]),
        gear(vec![
            mesh(vec![named(
                "materialOverride",
                list(vec![material("Form1Hair", 7)]),
            )]),
            enabled(),
            named("OverrideIdleEffects", list(vec![idle("Sun")])),
        ]),
    ]
}

fn swaps(n: usize) -> Vec<GearSwap> {
    (0..n)
        .map(|f| GearSwap {
            show: vec![h(&format!("Sword{f}"))],
            hide: Vec::new(),
            equip: None,
            transition: None,
        })
        .collect()
}

fn parsed_skin(bin: &[u8]) -> Vec<Field> {
    let file = parse_prop_file(bin).expect("prop");
    tree::parse_fields(&file.entries[0].body).expect("skin")
}

#[test]
fn test_each_form_keeps_its_own_idle_effects_and_materials() {
    let markers = [h("DekanForm1")];
    let mut forms = swaps(2);
    forms[1].show.push(h("Form1Hair"));
    let plan = material_plan(&diana(), &forms, &BTreeSet::new()).expect("plan");
    let out = apply_gears(
        &skin(vec![
            named("idleParticlesEffects", list(vec![idle("Base")])),
            mesh(Vec::new()),
        ]),
        &diana(),
        &markers,
        &plan,
    )
    .expect("apply")
    .expect("changed");
    let fields = parsed_skin(&out);
    assert!(
        tree::field(&fields, h("idleParticlesEffects")).is_none(),
        "idle now follows the active form"
    );
    let conditions = tree::field(&fields, h("PersistentEffectConditions"))
        .and_then(Value::items)
        .expect("conditions");
    assert_eq!(conditions.len(), 2);
    let second = conditions[1].fields().expect("condition");
    assert_eq!(
        tree::field(second, h("OwnerCondition")),
        Some(&form_active(1, &markers))
    );
    let vfx = tree::field(second, h("PersistentVfxs"))
        .and_then(Value::items)
        .expect("vfx");
    let vfx = vfx[0].fields().expect("vfx");
    assert_eq!(
        tree::field(vfx, h("effectKey")).and_then(Value::as_u32),
        Some(h("Sun"))
    );
    assert!(tree::field(vfx, h("mUnknownExtra")).is_none());

    let mesh_fields = tree::field(&fields, h("skinMeshProperties"))
        .and_then(Value::fields)
        .expect("mesh");
    let overrides = tree::field(mesh_fields, h("materialOverride"))
        .and_then(Value::items)
        .expect("overrides");
    assert_eq!(
        overrides,
        &[material("Form1Hair", 7)],
        "a part only its own form shows takes that form's material, with no copy"
    );
}

#[test]
fn test_a_form_without_override_keeps_the_skins_idle() {
    let gears = vec![
        gear(Vec::new()),
        gear(vec![
            enabled(),
            named("OverrideIdleEffects", list(vec![idle("Sun")])),
        ]),
    ];
    let out = apply_gears(
        &skin(vec![named(
            "idleParticlesEffects",
            list(vec![idle("Base")]),
        )]),
        &gears,
        &[h("DekanForm1")],
        &MaterialPlan::default(),
    )
    .expect("apply")
    .expect("changed");
    let conditions = tree::field(&parsed_skin(&out), h("PersistentEffectConditions"))
        .and_then(Value::items)
        .map(<[Value]>::to_vec)
        .expect("conditions");
    let first = tree::field(conditions[0].fields().expect("c"), h("PersistentVfxs"))
        .and_then(Value::items)
        .and_then(|v| v[0].fields())
        .and_then(|f| tree::field(f, h("effectKey")))
        .and_then(Value::as_u32);
    assert_eq!(first, Some(h("Base")));
    assert!(
        apply_gears(
            &skin(Vec::new()),
            &[gear(Vec::new()), gear(Vec::new())],
            &[h("DekanForm1")],
            &MaterialPlan::default()
        )
        .expect("apply")
        .is_none(),
        "gears with nothing per form leave the skin as it is"
    );
}

#[test]
fn test_coverage_reports_what_a_form_cycle_cannot_carry() {
    let mut gears = diana();
    gears.push(gear(vec![mesh(vec![
        named("materialOverride", list(vec![material("Form1Hair", 9)])),
        named("simpleSkin", text("other.skn")),
        named("texture", text("other.tex")),
    ])]));
    let buffed = pointer(
        "PersistentEffectConditionData",
        vec![named(
            "OwnerCondition",
            pointer("HasBuffDynamicMaterialBoolDriver", Vec::new()),
        )],
    );
    let source = skin(vec![named(
        "PersistentEffectConditions",
        Value::List {
            kind: FIELD_LIST2,
            element: FIELD_POINTER,
            items: vec![buffed, pointer("PersistentEffectConditionData", Vec::new())],
        },
    )]);
    let coverage = gear_coverage(&source, &gears, &swaps(3)).expect("coverage");
    assert_eq!(coverage.idle_forms, 2);
    assert_eq!(coverage.material_parts, 0);
    assert_eq!(
        coverage.part_copies, 2,
        "forms two and three draw the part with their own material"
    );
    assert!(
        coverage.mesh_swap,
        "another mesh per form cannot be shown by markers"
    );
    assert!(coverage.per_form_look.contains("texture"));
    assert_eq!(coverage.script_states, 1);
}

#[test]
fn test_a_part_drawn_with_another_material_per_form_gets_a_copy_for_that_form() {
    let gears = vec![
        gear(vec![mesh(vec![named(
            "materialOverride",
            list(vec![material("Hair", 1)]),
        )])]),
        gear(vec![mesh(vec![named(
            "materialOverride",
            list(vec![material("Hair", 2)]),
        )])]),
        gear(vec![mesh(vec![named(
            "materialOverride",
            list(vec![material("Hair", 1)]),
        )])]),
    ];
    let plan = material_plan(&gears, &swaps(3), &BTreeSet::new()).expect("plan");
    assert_eq!(plan.overrides, vec![material("Hair", 1)]);
    assert_eq!(
        plan.copies,
        vec![PartCopy {
            source: "Hair".into(),
            name: "Hair_F1".into(),
            form: 1,
            material: Some(material("Hair_F1", 2)),
        }],
        "the third form shares the first one's material and needs no copy"
    );
    let moved = with_copies(&swaps(3), &plan.copies);
    assert!(moved[1].show.contains(&h("Hair_F1")));
    assert!(moved[1].hide.contains(&h("Hair")));
    assert!(
        form_parts(&moved)[0].1.contains(&h("Hair_F1")),
        "other forms never show the copy"
    );

    let hidden_in_first = vec![
        GearSwap {
            show: vec![h("Sword0")],
            hide: vec![h("Hair")],
            equip: None,
            transition: None,
        },
        swaps(2)[1].clone(),
    ];
    let only_second = material_plan(&gears[..2], &hidden_in_first, &BTreeSet::new()).expect("plan");
    assert_eq!(only_second.overrides, vec![material("Hair", 2)]);
    assert!(
        only_second.copies.is_empty(),
        "a part hidden in a form does not need that form's material"
    );
}

#[test]
fn test_a_part_hidden_from_the_start_counts_only_where_a_form_shows_it() {
    let gears = vec![
        gear(Vec::new()),
        gear(vec![mesh(vec![named(
            "materialOverride",
            list(vec![material("Glow", 3)]),
        )])]),
    ];
    let mut forms = swaps(2);
    forms[1].show.push(h("Glow"));
    let hidden: BTreeSet<u32> = [h("Glow")].into_iter().collect();
    let plan = material_plan(&gears, &forms, &hidden).expect("plan");
    assert_eq!(plan.overrides, vec![material("Glow", 3)]);
    assert!(
        plan.copies.is_empty(),
        "the first form never shows the part, so it needs no copy"
    );
}
