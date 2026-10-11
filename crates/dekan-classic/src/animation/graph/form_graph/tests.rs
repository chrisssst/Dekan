use dekan_wad::prop::{PropEntry, PropFile};

use super::*;
use crate::form_marker::marker_names;

const GRAPH_KEY: u32 = 7;

fn hashes_of(fields: &[Field], name: &str) -> Vec<u32> {
    tree::field(fields, h(name))
        .and_then(Value::items)
        .map(|items| items.iter().filter_map(Value::as_u32).collect())
        .unwrap_or_default()
}

fn atomic(events: Vec<(u32, Value)>) -> Value {
    let mut fields = vec![
        named("mTrackDataName", hash_value(h("Default"))),
        named(
            "mAnimationResourceData",
            pointer("AnimationResourceData", Vec::new()),
        ),
    ];
    if !events.is_empty() {
        fields.push(named(
            "mEventDataMap",
            Value::Map {
                key: FIELD_HASH,
                value: FIELD_POINTER,
                entries: events
                    .into_iter()
                    .map(|(k, v)| (hash_value(k), v))
                    .collect(),
            },
        ));
    }
    pointer("AtomicClipData", fields)
}

fn sequence(children: &[&str]) -> Value {
    let keys: Vec<u32> = children.iter().map(|c| h(c)).collect();
    pointer(
        "SequencerClipData",
        vec![named("mClipNameList", hash_list(&keys))],
    )
}

fn map(entries: Vec<(u32, Value)>, value: u8) -> Value {
    Value::Map {
        key: FIELD_HASH,
        value,
        entries: entries
            .into_iter()
            .map(|(k, v)| (hash_value(k), v))
            .collect(),
    }
}

fn mask(joints: usize) -> Value {
    Value::Struct {
        kind: FIELD_EMBED,
        class: h("MaskData"),
        fields: vec![named(
            "mWeightList",
            Value::List {
                kind: FIELD_LIST,
                element: FIELD_F32,
                items: vec![
                    Value::Raw {
                        kind: FIELD_F32,
                        bytes: 1f32.to_le_bytes().to_vec()
                    };
                    joints
                ],
            },
        )],
    }
}

fn graph(masks: Vec<(u32, Value)>, clips: Vec<(&str, Value)>) -> Vec<u8> {
    let track = Value::Struct {
        kind: FIELD_EMBED,
        class: h("TrackData"),
        fields: Vec::new(),
    };
    serialize_prop_file(&PropFile {
        version: 3,
        links: Vec::new(),
        entries: vec![PropEntry {
            class_hash: h("AnimationGraphData"),
            key_hash: GRAPH_KEY,
            body: tree::write_fields(&[
                named(
                    "mTrackDataMap",
                    map(vec![(h("Default"), track)], FIELD_EMBED),
                ),
                named("mMaskDataMap", map(masks, FIELD_EMBED)),
                named(
                    "mClipDataMap",
                    map(
                        clips.into_iter().map(|(k, v)| (h(k), v)).collect(),
                        FIELD_POINTER,
                    ),
                ),
            ])
            .expect("body"),
        }],
    })
    .expect("graph")
}

fn swap(show: &str, equip: Option<&str>) -> GearSwap {
    GearSwap {
        show: vec![h(show)],
        hide: Vec::new(),
        equip: equip.map(str::to_owned),
        transition: None,
    }
}

fn three_swords() -> Vec<GearSwap> {
    vec![
        swap("Base_Sword", None),
        swap("Fighter_Sword", Some("Equip_Fighter")),
        swap("Tank_Sword", Some("Equip_Tank")),
    ]
}

fn markers() -> Vec<u32> {
    marker_names(3).iter().map(|name| h(name)).collect()
}

fn sword_graph() -> Vec<u8> {
    let recall_event = visibility_event(&[], &[h("Base_Sword"), h("Body")]);
    graph(
        vec![(h("Upper"), mask(3))],
        vec![
            ("Idle1", atomic(Vec::new())),
            ("Run", atomic(Vec::new())),
            ("Recall", atomic(vec![(1, recall_event)])),
            ("Spell1", sequence(&["Spell1_Cast", "Run"])),
            ("Spell1_Cast", atomic(Vec::new())),
            ("Equip_Fighter", atomic(Vec::new())),
            ("Equip_Tank", sequence(&["Spell1_Cast"])),
        ],
    )
}

fn clips(bin: &[u8]) -> BTreeMap<u32, Value> {
    let file = parse_prop_file(bin).expect("prop");
    let mut fields = tree::parse_fields(&file.entries[0].body).expect("fields");
    map_mut(&mut fields, "mClipDataMap")
        .expect("clips")
        .iter()
        .map(|(k, v)| (k.as_u32().expect("key"), v.clone()))
        .collect()
}

fn refs(clips: &BTreeMap<u32, Value>, key: u32) -> Vec<u32> {
    let mut out = Vec::new();
    clip_refs(clips.get(&key).expect("clip"), &mut out);
    out
}

fn swap_event(clip: &Value) -> (Vec<u32>, Vec<u32>) {
    let Some(Value::Map { entries, .. }) =
        tree::field(clip.fields().expect("clip"), h("mEventDataMap"))
    else {
        panic!("events");
    };
    let event = &entries
        .iter()
        .find(|(k, _)| k.as_u32() == Some(h("DekanGearSwap")))
        .expect("form event")
        .1;
    let fields = event.fields().expect("event");
    (
        hashes_of(fields, "mShowSubmeshList"),
        hashes_of(fields, "mHideSubmeshList"),
    )
}

fn built() -> BTreeMap<u32, Value> {
    let (bin, count) =
        build_form_graph(&sword_graph(), GRAPH_KEY, &three_swords(), &markers(), None)
            .expect("build")
            .expect("built");
    assert_eq!(count, 7);
    clips(&bin)
}

#[test]
fn test_every_clip_the_game_asks_for_picks_the_copy_of_the_form_marked_visible() {
    let clips = built();
    let run = h("Run");
    assert_eq!(refs(&clips, run), vec![copy_key(1, run), route_key(2, run)]);
    assert_eq!(
        refs(&clips, route_key(2, run)),
        vec![copy_key(2, run), copy_key(0, run)],
        "no marker visible is the first form"
    );
    assert_eq!(
        refs(&clips, copy_key(1, h("Spell1"))),
        vec![copy_key(1, h("Spell1_Cast")), copy_key(1, run)],
        "a composite copy stays inside its form, so a spell never changes form midway"
    );
}

#[test]
fn test_form_copies_show_their_parts_and_never_touch_a_marker() {
    let clips = built();
    let (show, hide) = swap_event(&clips[&copy_key(1, h("Run"))]);
    assert_eq!(show, vec![h("Fighter_Sword")]);
    assert_eq!(hide.len(), 2);
    assert!(hide.contains(&h("Base_Sword")) && hide.contains(&h("Tank_Sword")));
    assert!(
        markers()
            .iter()
            .all(|m| !show.contains(m) && !hide.contains(m))
    );

    let Some(Value::Map { entries, .. }) = tree::field(
        clips[&copy_key(2, h("Recall"))].fields().expect("recall"),
        h("mEventDataMap"),
    ) else {
        panic!("events");
    };
    let game = entries
        .iter()
        .find(|(k, _)| k.as_u32() == Some(1))
        .expect("game event");
    assert_eq!(
        hashes_of(game.1.fields().expect("event"), "mHideSubmeshList"),
        vec![h("Body")],
        "the game's own event keeps what is not a form part"
    );
}

#[test]
fn test_ctrl5_cycles_by_marker_and_holds_the_form_on_its_own_track() {
    let clips = built();
    let m = markers();
    let swap_key = |form: usize| h(&format!("DekanSwap{form}"));
    assert_eq!(
        refs(&clips, h("Toggle")),
        vec![swap_key(2), h("DekanToggle2")]
    );
    assert_eq!(
        refs(&clips, h("DekanToggle2")),
        vec![swap_key(0), swap_key(1)]
    );

    let state = h("DekanFormState1");
    let kick = h("DekanFormKick1");
    assert_eq!(refs(&clips, swap_key(1)), vec![state, kick]);
    let fields = clips[&state].fields().expect("state");
    assert_eq!(
        tree::field(fields, h("mTrackDataName")).and_then(Value::as_u32),
        Some(h(FORM_TRACK))
    );
    assert_eq!(
        tree::field(fields, h("mFlags")),
        Some(&Value::Raw {
            kind: FIELD_U32,
            bytes: HELD.to_le_bytes().to_vec()
        })
    );
    let (show, hide) = swap_event(&clips[&state]);
    assert_eq!(show, vec![h("Fighter_Sword"), m[0]]);
    assert!(hide.contains(&m[1]) && !hide.contains(&m[0]));
    assert_eq!(swap_event(&clips[&kick]), (show, hide));

    assert_eq!(
        refs(&clips, swap_key(2)),
        vec![h("DekanFormState2"), copy_key(2, h("Equip_Tank"))],
        "a composite equip animation plays its form copy"
    );
    let (base_show, base_hide) = swap_event(&clips[&h("DekanFormState0")]);
    assert_eq!(base_show, vec![h("Base_Sword")]);
    assert!(m.iter().all(|marker| base_hide.contains(marker)));
}

#[test]
fn test_the_built_graph_resolves_and_adds_a_pose_free_track() {
    let (bin, _) = build_form_graph(&sword_graph(), GRAPH_KEY, &three_swords(), &markers(), None)
        .expect("build")
        .expect("built");
    let file = parse_prop_file(&bin).expect("prop");
    let mut fields = tree::parse_fields(&file.entries[0].body).expect("fields");
    assert!(every_reference_resolves(&mut fields));
    assert!(keys_of(&mut fields, "mTrackDataMap").contains(&h(FORM_TRACK)));
    let no_pose = map_mut(&mut fields, "mMaskDataMap")
        .expect("masks")
        .iter()
        .find(|(k, _)| k.as_u32() == Some(h(NO_POSE_MASK)))
        .map(|(_, v)| v.clone())
        .expect("mask");
    let weights = tree::field(no_pose.fields().expect("mask"), h("mWeightList"))
        .and_then(Value::items)
        .expect("weights")
        .to_vec();
    assert_eq!(weights.len(), 3);
    assert!(weights.iter().all(|w| *w
        == Value::Raw {
            kind: FIELD_F32,
            bytes: vec![0; 4]
        }));
}

#[test]
fn test_graphs_that_cannot_hold_forms_are_left_alone() {
    let swords = three_swords();
    let m = markers();
    let build = |bin: &[u8], swaps: &[GearSwap], markers: &[u32]| {
        build_form_graph(bin, GRAPH_KEY, swaps, markers, None).expect("build")
    };
    assert!(build(&sword_graph(), &swords[..1], &[]).is_none());
    assert!(build(&sword_graph(), &swords, &m[..1]).is_none());
    assert!(
        build_form_graph(&sword_graph(), 99, &swords, &m, None)
            .expect("build")
            .is_none()
    );
    let no_mask = graph(Vec::new(), vec![("Idle1", atomic(Vec::new()))]);
    assert!(build(&no_mask, &swords, &m).is_none());
    let toggle = graph(
        vec![(h("Upper"), mask(3))],
        vec![
            ("Idle1", atomic(Vec::new())),
            ("Toggle", atomic(Vec::new())),
        ],
    );
    assert!(build(&toggle, &swords, &m).is_none());
    let no_carrier = graph(
        vec![(h("Upper"), mask(3))],
        vec![("Run", atomic(Vec::new()))],
    );
    assert!(build(&no_carrier, &swords, &m).is_none());
    let dangling = graph(
        vec![(h("Upper"), mask(3))],
        vec![("Idle1", atomic(Vec::new())), ("Seq", sequence(&["Gone"]))],
    );
    assert!(build(&dangling, &swords, &m).is_none());
}

fn mask_lengths(bin: &[u8]) -> Vec<(u32, usize)> {
    let file = parse_prop_file(bin).expect("prop");
    let mut fields = tree::parse_fields(&file.entries[0].body).expect("fields");
    map_mut(&mut fields, "mMaskDataMap")
        .expect("masks")
        .iter()
        .map(|(k, v)| {
            let len = v
                .fields()
                .and_then(|f| tree::field(f, h("mWeightList")))
                .and_then(Value::items)
                .map_or(0, <[Value]>::len);
            (k.as_u32().expect("key"), len)
        })
        .collect()
}

#[test]
fn a_graph_without_masks_gets_the_no_pose_mask_sized_by_the_merged_skeleton() {
    let clips = vec![("Idle1", atomic(Vec::new())), ("Run", atomic(Vec::new()))];
    let maskless = graph(Vec::new(), clips);
    let swords = three_swords();
    assert!(
        build_form_graph(&maskless, GRAPH_KEY, &swords, &markers(), None)
            .expect("build")
            .is_none(),
        "without masks or a skeleton the joint count is unknown"
    );
    let (bin, _) = build_form_graph(&maskless, GRAPH_KEY, &swords, &markers(), Some(5))
        .expect("build")
        .expect("cycle");
    assert_eq!(mask_lengths(&bin), [(h(NO_POSE_MASK), 5)]);
}

#[test]
fn masks_shorter_than_the_merged_skeleton_are_padded_so_twin_joints_are_covered() {
    let (bin, _) = build_form_graph(
        &sword_graph(),
        GRAPH_KEY,
        &three_swords(),
        &markers(),
        Some(6),
    )
    .expect("build")
    .expect("cycle");
    assert_eq!(mask_lengths(&bin), [(h("Upper"), 6), (h(NO_POSE_MASK), 6)]);
    let (same, _) = build_form_graph(&sword_graph(), GRAPH_KEY, &three_swords(), &markers(), None)
        .expect("build")
        .expect("cycle");
    assert_eq!(mask_lengths(&same), [(h("Upper"), 3), (h(NO_POSE_MASK), 3)]);
}
