use dekan_wad::prop::{PropEntry, PropFile};

use super::*;
use crate::gear_toggle::hash_value;

fn hashes_of(fields: &[tree::Field], name: &str) -> Vec<u32> {
    tree::field(fields, h(name))
        .and_then(Value::items)
        .map(|items| items.iter().filter_map(Value::as_u32).collect())
        .unwrap_or_default()
}

fn swap(show: &str, hide: &[&str]) -> GearSwap {
    GearSwap {
        show: vec![h(show)],
        hide: hide.iter().map(|p| h(p)).collect(),
        equip: None,
        transition: None,
    }
}

fn swords() -> Vec<GearSwap> {
    vec![
        swap("Base_Sword", &[]),
        swap("Fighter_Sword", &["Glow"]),
        swap("Tank_Sword", &[]),
    ]
}

fn markers() -> Vec<u32> {
    vec![h("DekanForm1"), h("DekanForm2")]
}

fn skin_bin(conditions: Option<Vec<Value>>) -> Vec<u8> {
    let mut fields = vec![named("mName", hash_value(1))];
    if let Some(items) = conditions {
        fields.push(named(
            "PersistentEffectConditions",
            Value::List {
                kind: FIELD_LIST2,
                element: FIELD_POINTER,
                items,
            },
        ));
    }
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

fn conditions(bin: &[u8]) -> Vec<Value> {
    let file = parse_prop_file(bin).expect("prop");
    let fields = tree::parse_fields(&file.entries[0].body).expect("fields");
    tree::field(&fields, h("PersistentEffectConditions"))
        .and_then(Value::items)
        .map(<[Value]>::to_vec)
        .expect("conditions")
}

#[test]
fn test_a_form_is_active_while_its_state_clip_plays_or_its_marker_is_visible() {
    let m = markers();
    let active = form_active(1, &m);
    assert_eq!(active.class(), Some(h("OneTrueMaterialDriver")));
    let drivers = tree::field(active.fields().expect("one"), h("mDrivers"))
        .and_then(Value::items)
        .expect("drivers");
    assert_eq!(drivers.len(), 2);
    assert_eq!(
        drivers[0].class(),
        Some(h("IsAnimationPlayingDynamicMaterialBoolDriver"))
    );
    assert_eq!(
        hashes_of(drivers[0].fields().expect("playing"), "mAnimationNames"),
        vec![state_key(1), swap_key(1)]
    );
    assert_eq!(drivers[1], part_visible(FIELD_POINTER, m[0]));

    let base = form_active(0, &m);
    assert_eq!(base.class(), Some(h("NotMaterialDriver")));
    let inner = tree::field(base.fields().expect("not"), h("mDriver")).expect("inner");
    assert_eq!(
        tree::field(inner.fields().expect("one"), h("mDrivers"))
            .and_then(Value::items)
            .map(<[Value]>::len),
        Some(2),
        "the first form is no other form active"
    );
    assert_eq!(form_drivers(&m).len(), 3);
}

#[test]
fn test_each_form_holds_its_parts_and_marker_through_a_persistent_condition() {
    let out = persist_forms(
        &skin_bin(Some(vec![pointer("Existing", Vec::new())])),
        &swords(),
        &markers(),
    )
    .expect("persist")
    .expect("changed");
    let added = conditions(&out);
    assert_eq!(added.len(), 4, "the game's own condition stays first");
    assert_eq!(added[0].class(), Some(h("Existing")));
    let fighter = added[2].fields().expect("condition");
    assert_eq!(
        tree::field(fighter, h("OwnerCondition")),
        Some(&form_active(1, &markers()))
    );
    assert_eq!(
        hashes_of(fighter, "SubmeshesToShow"),
        vec![h("Fighter_Sword"), h("DekanForm1")]
    );
    let hidden = hashes_of(fighter, "SubmeshesToHide");
    assert!(
        [h("Base_Sword"), h("Tank_Sword"), h("Glow"), h("DekanForm2")]
            .iter()
            .all(|p| hidden.contains(p))
    );
    assert!(!hidden.contains(&h("Fighter_Sword")) && !hidden.contains(&h("DekanForm1")));
    let base = added[1].fields().expect("base");
    assert_eq!(hashes_of(base, "SubmeshesToShow"), vec![h("Base_Sword")]);
    assert!(hashes_of(base, "SubmeshesToHide").contains(&h("DekanForm1")));

    let fresh = persist_forms(&skin_bin(None), &swords(), &markers())
        .expect("persist")
        .expect("changed");
    assert_eq!(conditions(&fresh).len(), 3);
}

#[test]
fn test_skins_without_forms_or_skin_object_are_left_alone() {
    assert!(
        persist_forms(&skin_bin(None), &swords()[..1], &[])
            .expect("persist")
            .is_none()
    );
    assert!(
        persist_forms(&skin_bin(None), &swords(), &markers()[..1])
            .expect("persist")
            .is_none()
    );
    let no_skin = serialize_prop_file(&PropFile {
        version: 3,
        links: Vec::new(),
        entries: Vec::new(),
    })
    .expect("bin");
    assert!(
        persist_forms(&no_skin, &swords(), &markers())
            .expect("persist")
            .is_none()
    );
}
