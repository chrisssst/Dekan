use dekan_wad::prop::tree::{self, Field, Value};

use crate::form_state::form_active;
use crate::gear_toggle::{h, hash_value, named, pointer};

pub(crate) const FIELD_U8: u8 = 3;
pub(crate) const FIELD_I16: u8 = 4;
pub(crate) const FIELD_F32: u8 = 10;
pub(crate) const FIELD_VEC3: u8 = 12;
pub(crate) const FIELD_VEC4: u8 = 13;
pub(crate) const FIELD_STRING: u8 = 16;
pub(crate) const FIELD_LIST: u8 = 0x80;
pub(crate) const FIELD_LIST2: u8 = 0x81;
pub(crate) const FIELD_POINTER: u8 = 0x82;
pub(crate) const FIELD_EMBED: u8 = 0x83;
pub(crate) const FIELD_LINK: u8 = 0x84;
pub(crate) const FIELD_FLAG: u8 = 0x87;
pub(crate) const FORM_STENCILS: [u8; 10] = [42, 43, 44, 45, 46, 47, 48, 54, 55, 57];
pub(crate) const STENCIL_WRITE: u8 = 1;
pub(crate) const STENCIL_EQUAL: u8 = 2;
pub(crate) const EMITTER_LISTS: [&str; 2] = [
    "complexEmitterDefinitionData",
    "simpleEmitterDefinitionData",
];
pub(crate) const WHITE: &str = "assets/shared/materials/white.tex";
pub(crate) const GROUND_BONE: &str = "buffbone_glb_ground_loc";
pub(crate) const MAX_DEPTH: usize = 16;

pub(crate) fn form_stencil(form: usize) -> u8 {
    FORM_STENCILS[form.min(FORM_STENCILS.len() - 1)]
}

pub(crate) fn raw(kind: u8, bytes: Vec<u8>) -> Value {
    Value::Raw { kind, bytes }
}

pub(crate) fn text(value: &str) -> Value {
    let body = &value.as_bytes()[..value.len().min(usize::from(u16::MAX))];
    let mut bytes = u16::try_from(body.len())
        .unwrap_or(u16::MAX)
        .to_le_bytes()
        .to_vec();
    bytes.extend_from_slice(body);
    raw(FIELD_STRING, bytes)
}

pub(crate) fn raw_text(value: &Value) -> Option<String> {
    match value {
        Value::Raw {
            kind: FIELD_STRING,
            bytes,
        } => bytes
            .get(2..)
            .map(|text| String::from_utf8_lossy(text).into_owned()),
        _ => None,
    }
}

pub(crate) fn walk(value: &Value, visit: &mut impl FnMut(&Value)) {
    visit(value);
    match value {
        Value::Struct { fields, .. } => fields.iter().for_each(|f| walk(&f.value, visit)),
        Value::List { items, .. } => items.iter().for_each(|v| walk(v, visit)),
        Value::Map { entries, .. } => entries.iter().for_each(|(_, v)| walk(v, visit)),
        Value::Optional { value: Some(v), .. } => walk(v, visit),
        _ => {}
    }
}

pub(crate) fn embed(class: &str, fields: Vec<Field>) -> Value {
    Value::Struct {
        kind: FIELD_EMBED,
        class: h(class),
        fields,
    }
}

pub(crate) fn constant(class: &str, kind: u8, bytes: Vec<u8>) -> Value {
    embed(class, vec![named("constantValue", raw(kind, bytes))])
}

pub(crate) fn floats(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

pub fn ground_bone(skl: &[u8]) -> Option<String> {
    let lower = skl.to_ascii_lowercase();
    let at = lower
        .windows(GROUND_BONE.len())
        .position(|w| w == GROUND_BONE.as_bytes())?;
    String::from_utf8(skl[at..at + GROUND_BONE.len()].to_vec()).ok()
}

fn marker_system(form: usize, on_load: bool) -> Vec<Field> {
    let mut emitter = vec![
        named("emitterName", text(&format!("DekanMarker{form}"))),
        named("rate", constant("ValueFloat", FIELD_F32, floats(&[1.0]))),
        named(
            "particleLifetime",
            constant(
                "ValueFloat",
                FIELD_F32,
                floats(&[if on_load { 2.5 } else { 1e14 }]),
            ),
        ),
        named("importance", raw(FIELD_U8, vec![3])),
        named("blendMode", raw(FIELD_U8, vec![4])),
        named(
            "pass",
            raw(
                FIELD_I16,
                (if form == 0 { -9999i16 } else { -9998 })
                    .to_le_bytes()
                    .to_vec(),
            ),
        ),
        named("miscRenderFlags", raw(FIELD_U8, vec![1])),
        named("stencilMode", raw(FIELD_U8, vec![STENCIL_WRITE])),
        named("stencilRef", raw(FIELD_U8, vec![form_stencil(form)])),
        named("isGroundLayer", raw(FIELD_FLAG, vec![1])),
        named("isUniformScale", raw(FIELD_FLAG, vec![1])),
        named(
            "birthScale0",
            constant(
                "ValueVector3",
                FIELD_VEC3,
                floats(&[4000.0, 4000.0, 4000.0]),
            ),
        ),
        named(
            "birthColor",
            constant("ValueColor", FIELD_VEC4, floats(&[0.0, 0.0, 0.0, 1.0])),
        ),
        named("texture", text(WHITE)),
    ];
    if on_load {
        emitter.push(named(
            "timeBeforeFirstEmission",
            raw(FIELD_F32, floats(&[1.0])),
        ));
    } else {
        emitter.push(named("isSingleParticle", raw(FIELD_FLAG, vec![1])));
    }
    let name = format!(
        "Dekan/FormMarker{form}{}",
        if on_load { "Load" } else { "" }
    );
    vec![
        named(
            "complexEmitterDefinitionData",
            Value::List {
                kind: FIELD_LIST,
                element: FIELD_POINTER,
                items: vec![pointer("VfxEmitterDefinitionData", emitter)],
            },
        ),
        named("particleName", text(&name)),
        named("particlePath", text(&name)),
        named("visibilityRadius", raw(FIELD_F32, floats(&[10000.0]))),
        named("objectPath", hash_value(h(&name))),
    ]
}

pub(crate) fn marker_systems(forms: usize) -> Vec<(u32, u32, Vec<Field>)> {
    let mut systems: Vec<(u32, u32, Vec<Field>)> = (0..forms)
        .map(|form| {
            (
                h(&format!("DekanMarker{form}")),
                h(&format!("Dekan/FormMarker{form}")),
                marker_system(form, false),
            )
        })
        .collect();
    systems.push((
        h("DekanMarker0Load"),
        h("Dekan/FormMarker0Load"),
        marker_system(0, true),
    ));
    systems
}

pub(crate) fn hold_markers(skin: &mut Vec<Field>, forms: usize, markers: &[u32], bone_name: &str) {
    let bone = || named("boneName", text(bone_name));
    let conditions = (0..forms).map(|form| {
        pointer(
            "PersistentEffectConditionData",
            vec![
                named("OwnerCondition", form_active(form, markers)),
                named(
                    "PersistentVfxs",
                    Value::List {
                        kind: FIELD_LIST2,
                        element: FIELD_EMBED,
                        items: vec![embed(
                            "PersistentVfxData",
                            vec![
                                bone(),
                                named("effectKey", hash_value(h(&format!("DekanMarker{form}")))),
                            ],
                        )],
                    },
                ),
            ],
        )
    });
    match tree::field_mut(skin, h("PersistentEffectConditions")).and_then(Value::items_mut) {
        Some(items) => items.extend(conditions),
        None => tree::set_field(
            skin,
            h("PersistentEffectConditions"),
            Value::List {
                kind: FIELD_LIST2,
                element: FIELD_POINTER,
                items: conditions.collect(),
            },
        ),
    }
    let idle = embed(
        "SkinCharacterDataProperties_CharacterIdleEffect",
        vec![
            named("effectKey", hash_value(h("DekanMarker0Load"))),
            bone(),
        ],
    );
    match tree::field_mut(skin, h("idleParticlesEffects")).and_then(Value::items_mut) {
        Some(items) => items.push(idle),
        None => tree::set_field(
            skin,
            h("idleParticlesEffects"),
            Value::List {
                kind: FIELD_LIST,
                element: FIELD_EMBED,
                items: vec![idle],
            },
        ),
    }
}
