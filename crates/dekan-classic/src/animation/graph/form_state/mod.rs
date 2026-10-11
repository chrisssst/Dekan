use std::collections::BTreeSet;

use dekan_wad::prop::tree::{self, Value};
use dekan_wad::prop::{parse_prop_file, serialize_prop_file};

use crate::error::ClassicError;
use crate::gear_toggle::{
    FIELD_HASH, GearSwap, bin_error, h, hash_list, named, part_visible, pointer,
};

const FIELD_LIST: u8 = 0x80;
const FIELD_LIST2: u8 = 0x81;
const FIELD_POINTER: u8 = 0x82;
const SKIN_CLASS: &str = "SkinCharacterDataProperties";

#[must_use]
pub fn state_key(form: usize) -> u32 {
    h(&format!("DekanFormState{form}"))
}

#[must_use]
pub fn swap_key(form: usize) -> u32 {
    h(&format!("DekanSwap{form}"))
}

#[must_use]
pub fn kick_key(form: usize) -> u32 {
    h(&format!("DekanFormKick{form}"))
}

fn drivers(items: Vec<Value>) -> Value {
    Value::List {
        kind: FIELD_LIST,
        element: FIELD_POINTER,
        items,
    }
}

fn one_true(items: Vec<Value>) -> Value {
    pointer(
        "OneTrueMaterialDriver",
        vec![named("mDrivers", drivers(items))],
    )
}

#[must_use]
pub fn form_active(form: usize, markers: &[u32]) -> Value {
    if form == 0 {
        let others = (1..=markers.len())
            .map(|f| form_active(f, markers))
            .collect();
        return pointer(
            "NotMaterialDriver",
            vec![named("mDriver", one_true(others))],
        );
    }
    let playing = pointer(
        "IsAnimationPlayingDynamicMaterialBoolDriver",
        vec![named(
            "mAnimationNames",
            hash_list(&[state_key(form), swap_key(form)]),
        )],
    );
    one_true(vec![
        playing,
        part_visible(FIELD_POINTER, markers[form - 1]),
    ])
}

#[must_use]
pub fn form_drivers(markers: &[u32]) -> Vec<Value> {
    (0..=markers.len())
        .map(|form| form_active(form, markers))
        .collect()
}

#[must_use]
pub fn form_parts(swaps: &[GearSwap]) -> Vec<(Vec<u32>, Vec<u32>)> {
    let parts: BTreeSet<u32> = swaps.iter().flat_map(|s| s.show.iter().copied()).collect();
    swaps
        .iter()
        .map(|swap| {
            let hide = parts
                .iter()
                .chain(swap.hide.iter())
                .copied()
                .filter(|part| !swap.show.contains(part))
                .collect::<BTreeSet<u32>>()
                .into_iter()
                .collect();
            (swap.show.clone(), hide)
        })
        .collect()
}

fn hashes2(list: &[u32]) -> Value {
    let Value::List { items, .. } = hash_list(list) else {
        return hash_list(list);
    };
    Value::List {
        kind: FIELD_LIST2,
        element: FIELD_HASH,
        items,
    }
}

pub fn persist_forms(
    skin_bin: &[u8],
    swaps: &[GearSwap],
    markers: &[u32],
) -> Result<Option<Vec<u8>>, ClassicError> {
    if swaps.len() < 2 || markers.len() + 1 != swaps.len() {
        return Ok(None);
    }
    let mut file = parse_prop_file(skin_bin).map_err(bin_error)?;
    let Some(at) = file
        .entries
        .iter()
        .position(|e| e.class_hash == h(SKIN_CLASS))
    else {
        return Ok(None);
    };
    let mut fields = tree::parse_fields(&file.entries[at].body).map_err(bin_error)?;
    let conditions: Vec<Value> = form_parts(swaps)
        .into_iter()
        .enumerate()
        .map(|(form, (mut show, mut hide))| {
            show.extend(form.checked_sub(1).map(|m| markers[m]));
            hide.extend(
                markers
                    .iter()
                    .enumerate()
                    .filter(|(m, _)| m + 1 != form)
                    .map(|(_, marker)| *marker),
            );
            pointer(
                "PersistentEffectConditionData",
                vec![
                    named("OwnerCondition", form_active(form, markers)),
                    named("SubmeshesToShow", hashes2(&show)),
                    named("SubmeshesToHide", hashes2(&hide)),
                ],
            )
        })
        .collect();
    match tree::field_mut(&mut fields, h("PersistentEffectConditions")).and_then(Value::items_mut) {
        Some(items) => items.extend(conditions),
        None => tree::set_field(
            &mut fields,
            h("PersistentEffectConditions"),
            Value::List {
                kind: FIELD_LIST2,
                element: FIELD_POINTER,
                items: conditions,
            },
        ),
    }
    file.entries[at].body = tree::write_fields(&fields).map_err(bin_error)?;
    serialize_prop_file(&file).map(Some).map_err(bin_error)
}

#[cfg(test)]
mod tests;
