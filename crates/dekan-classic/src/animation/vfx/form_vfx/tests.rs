use dekan_wad::prop::PropFile;

use super::*;
use crate::form_state::form_active;
use crate::gear_toggle::{FIELD_HASH, pointer};
use crate::vfx_markers::{FIELD_EMBED, STENCIL_WRITE, ground_bone};

const KEY: &str = "Viego_R_Sword";

fn emitter(name: &str, extra: Vec<Field>) -> Value {
    let mut fields = vec![named("emitterName", text(name))];
    fields.extend(extra);
    pointer("VfxEmitterDefinitionData", fields)
}

fn system(emitters: Vec<Value>) -> Vec<Field> {
    vec![
        named(
            "complexEmitterDefinitionData",
            Value::List {
                kind: FIELD_LIST,
                element: FIELD_POINTER,
                items: emitters,
            },
        ),
        named("particleName", text("Original")),
        named("objectPath", hash_value(1)),
    ]
}

fn child_of(path: u32) -> Field {
    named(
        "childParticleSetDefinition",
        pointer(
            "VfxChildParticleSetDefinitionData",
            vec![named(
                "childrenIdentifiers",
                Value::List {
                    kind: FIELD_LIST,
                    element: FIELD_EMBED,
                    items: vec![embed(
                        "VfxChildIdentifier",
                        vec![named(
                            "effect",
                            raw(FIELD_LINK, path.to_le_bytes().to_vec()),
                        )],
                    )],
                },
            )],
        ),
    )
}

fn entry(class: &str, key: u32, fields: Vec<Field>) -> PropEntry {
    PropEntry {
        class_hash: h(class),
        key_hash: key,
        body: tree::write_fields(&fields).expect("body"),
    }
}

fn bin(entries: Vec<PropEntry>) -> Vec<u8> {
    serialize_prop_file(&PropFile {
        version: 3,
        links: Vec::new(),
        entries,
    })
    .expect("bin")
}

fn resolver(pairs: &[(u32, u32)]) -> Vec<Field> {
    vec![named(
        "resourceMap",
        Value::Map {
            key: FIELD_HASH,
            value: FIELD_LINK,
            entries: pairs
                .iter()
                .map(|(k, v)| (hash_value(*k), raw(FIELD_LINK, v.to_le_bytes().to_vec())))
                .collect(),
        },
    )]
}

fn gear(pairs: &[(u32, u32)]) -> Vec<u8> {
    tree::write_fields(&[named(
        "mGearData",
        embed(
            "GearData",
            vec![named(
                "mVFXResourceResolver",
                embed("ResourceResolver", resolver(pairs)),
            )],
        ),
    )])
    .expect("gear")
}

const RED: u32 = 0x100;
const BLUE: u32 = 0x200;
const SPARK: u32 = 0x300;

fn source() -> Vec<u8> {
    let shared = emitter("Glow", Vec::new());
    bin(vec![
        entry(
            "VfxSystemDefinitionData",
            RED,
            system(vec![shared.clone(), emitter("RedBlade", Vec::new())]),
        ),
        entry(
            "VfxSystemDefinitionData",
            BLUE,
            system(vec![
                shared,
                emitter(
                    "BlueBlade",
                    vec![named("StencilReferenceId", hash_value(9)), child_of(SPARK)],
                ),
            ]),
        ),
        entry(
            "VfxSystemDefinitionData",
            SPARK,
            system(vec![emitter("Spark", Vec::new())]),
        ),
    ])
}

fn skin0() -> Vec<u8> {
    bin(vec![
        entry(
            "SkinCharacterDataProperties",
            1,
            vec![named(
                "mResourceResolver",
                raw(FIELD_LINK, 2u32.to_le_bytes().to_vec()),
            )],
        ),
        entry(
            "ResourceResolver",
            2,
            resolver(&[(h(KEY), RED), (h("Other"), 0x900)]),
        ),
    ])
}

fn plan_for<'a>(bodies: &'a [Vec<u8>], src: &'a [Vec<u8>], markers: &'a [u32]) -> FormEffects<'a> {
    FormEffects {
        bins: src,
        gear_bodies: bodies,
        markers,
        bone: "Buffbone_Glb_Ground_Loc",
    }
}

fn systems_of(out: &[u8]) -> HashMap<u32, Vec<Field>> {
    parse_prop_file(out)
        .expect("prop")
        .entries
        .iter()
        .filter(|e| e.class_hash == h("VfxSystemDefinitionData"))
        .map(|e| (e.key_hash, tree::parse_fields(&e.body).expect("fields")))
        .collect()
}

fn emitters_of(fields: &[Field]) -> Vec<Vec<Field>> {
    tree::field(fields, h("complexEmitterDefinitionData"))
        .and_then(Value::items)
        .expect("emitters")
        .iter()
        .map(|e| e.fields().expect("emitter").to_vec())
        .collect()
}

fn stencil(emitter: &[Field]) -> Option<(u8, u8)> {
    let byte = |name: &str| match tree::field(emitter, h(name)) {
        Some(Value::Raw { bytes, .. }) => bytes.first().copied(),
        _ => None,
    };
    Some((byte("stencilMode")?, byte("stencilRef")?))
}

fn resolver_of(out: &[u8]) -> BTreeMap<u32, u32> {
    let file = parse_prop_file(out).expect("prop");
    let r = file
        .entries
        .iter()
        .find(|e| e.key_hash == 2)
        .expect("resolver");
    resource_map(&embed(
        "ResourceResolver",
        tree::parse_fields(&r.body).expect("fields"),
    ))
}

#[test]
fn test_an_effect_that_changes_by_form_draws_each_form_only_under_its_stencil() {
    let bodies = vec![gear(&[(h(KEY), RED)]), gear(&[(h(KEY), BLUE)])];
    let src = vec![source()];
    let markers = [h("DekanForm1")];
    let (out, stamped) = gate_form_effects(&skin0(), &plan_for(&bodies, &src, &markers))
        .expect("gate")
        .expect("gated");
    let map = resolver_of(&out);
    assert_eq!(
        map[&h("Other")],
        0x900,
        "keys no form changes stay as the game wrote them"
    );
    let merged_path = map[&h(KEY)];
    let systems = systems_of(&out);
    let merged = emitters_of(&systems[&merged_path]);
    assert_eq!(merged.len(), 3, "the emitter both forms share goes in once");
    assert_eq!(stencil(&merged[0]), None);
    assert_eq!(stencil(&merged[1]), Some((STENCIL_EQUAL, STENCIL_BASE)));
    assert_eq!(stencil(&merged[2]), Some((STENCIL_EQUAL, STENCIL_BASE + 1)));
    assert!(
        tree::field(&merged[2], h("StencilReferenceId")).is_none(),
        "a named stencil would replace the form's reference"
    );
    let child = tree::field(&merged[2], h("childParticleSetDefinition"))
        .and_then(Value::fields)
        .and_then(|c| tree::field(c, h("childrenIdentifiers")))
        .and_then(Value::items)
        .and_then(|ids| ids[0].fields())
        .and_then(|id| tree::field(id, h("effect")))
        .and_then(Value::as_u32)
        .expect("child link");
    assert_eq!(child, gated_key(SPARK, 1));
    assert_eq!(
        stencil(&emitters_of(&systems[&child])[0]),
        Some((STENCIL_EQUAL, STENCIL_BASE + 1)),
        "a child effect follows its parent's form"
    );
    assert_eq!(stamped, 3);
}

#[test]
fn test_each_form_draws_its_stencil_marker_while_active() {
    let bodies = vec![gear(&[(h(KEY), RED)]), gear(&[(h(KEY), BLUE)])];
    let src = vec![source()];
    let markers = [h("DekanForm1")];
    let (out, _) = gate_form_effects(&skin0(), &plan_for(&bodies, &src, &markers))
        .expect("gate")
        .expect("gated");
    let map = resolver_of(&out);
    let systems = systems_of(&out);
    for form in 0..2 {
        let marker = emitters_of(&systems[&map[&h(&format!("DekanMarker{form}"))]]);
        assert_eq!(
            stencil(&marker[0]),
            Some((
                STENCIL_WRITE,
                STENCIL_BASE + u8::try_from(form).expect("form")
            ))
        );
    }
    let load = emitters_of(&systems[&map[&h("DekanMarker0Load")]]);
    assert!(tree::field(&load[0], h("timeBeforeFirstEmission")).is_some());

    let file = parse_prop_file(&out).expect("prop");
    let skin = tree::parse_fields(&file.entries[0].body).expect("skin");
    let conditions = tree::field(&skin, h("PersistentEffectConditions"))
        .and_then(Value::items)
        .expect("conditions");
    assert_eq!(conditions.len(), 2);
    assert_eq!(
        tree::field(conditions[1].fields().expect("c"), h("OwnerCondition")),
        Some(&form_active(1, &markers))
    );
    let idle = tree::field(&skin, h("idleParticlesEffects"))
        .and_then(Value::items)
        .expect("idle");
    assert_eq!(
        tree::field(idle[0].fields().expect("idle"), h("effectKey")).and_then(Value::as_u32),
        Some(h("DekanMarker0Load"))
    );
}

#[test]
fn test_skins_whose_effects_do_not_change_by_form_are_left_alone() {
    let src = vec![source()];
    let markers = [h("DekanForm1")];
    let same = vec![gear(&[(h(KEY), RED)]), gear(&[(h(KEY), RED)])];
    assert!(
        gate_form_effects(&skin0(), &plan_for(&same, &src, &markers))
            .expect("gate")
            .is_none()
    );
    let one = vec![gear(&[(h(KEY), RED)])];
    assert!(
        gate_form_effects(&skin0(), &plan_for(&one, &src, &[]))
            .expect("gate")
            .is_none()
    );
    let no_resolver = bin(vec![entry("SkinCharacterDataProperties", 1, Vec::new())]);
    let bodies = vec![gear(&[(h(KEY), RED)]), gear(&[(h(KEY), BLUE)])];
    assert!(
        gate_form_effects(&no_resolver, &plan_for(&bodies, &src, &markers))
            .expect("gate")
            .is_none()
    );
}

#[test]
fn test_the_ground_bone_keeps_the_skeleton_spelling() {
    let skl = b"\0\0Root\0Buffbone_Glb_Ground_Loc\0".to_vec();
    assert_eq!(
        ground_bone(&skl).as_deref(),
        Some("Buffbone_Glb_Ground_Loc")
    );
    assert_eq!(ground_bone(b"Root\0Spine"), None);
}
