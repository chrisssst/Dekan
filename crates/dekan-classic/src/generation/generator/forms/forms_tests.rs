use super::tests::*;
use super::*;
use crate::gear_toggle::{h, hash_value, named, pointer};
use crate::mesh::skeleton::tests::{joint, skeleton};
use crate::mesh::skinned_mesh::tests::mesh;
use crate::vfx_markers::{embed, text};
use dekan_wad::prop::tree::{self, Field, Value};

pub(crate) const SKIN: u32 = 7;
pub(crate) const BANK: &str = "ASSETS/Sounds/Zed_Skin07_SFX_events.bnk";
pub(crate) const TRANSFORM: &str = "ZedStormTransform";
const LINK: u8 = 0x84;
const FILE: u8 = 18;

fn link(path: &str) -> Value {
    Value::Raw {
        kind: LINK,
        bytes: h(path).to_le_bytes().to_vec(),
    }
}

fn file(hash: u64) -> Value {
    Value::Raw {
        kind: FILE,
        bytes: hash.to_le_bytes().to_vec(),
    }
}

fn model(mesh: &str, skeleton: &str, texture: u64) -> Value {
    embed(
        "SkinMeshDataProperties",
        vec![
            named("skeleton", text(skeleton)),
            named("simpleSkin", text(mesh)),
            named("texture", file(texture)),
        ],
    )
}

fn entry(class: &str, key: &str, fields: Vec<Field>) -> PropEntry {
    PropEntry {
        class_hash: h(class),
        key_hash: h(key),
        body: tree::write_fields(&fields).expect("body"),
    }
}

fn elementalist_skin() -> Vec<u8> {
    let prefix = format!("Characters/Zed/Skins/Skin{SKIN}");
    let gears = [
        format!("{prefix}/Gear/Light"),
        format!("{prefix}/Gear/Storm"),
    ];
    let skin = entry(
        "SkinCharacterDataProperties",
        &prefix,
        vec![
            named("objectPath", hash_value(h(&prefix))),
            named(
                "skinMeshProperties",
                model("ASSETS/Zed/Light.skn", "ASSETS/Zed/Light.skl", 1),
            ),
            named(
                "skinAnimationProperties",
                embed(
                    "SkinAnimationProperties",
                    vec![named(
                        "animationGraphData",
                        link(&format!("Characters/Zed/Animations/Skin{SKIN}")),
                    )],
                ),
            ),
            named(
                "skinAudioProperties",
                embed(
                    "SkinAudioProperties",
                    vec![named(
                        "bankUnits",
                        Value::List {
                            kind: 0x80,
                            element: 0x83,
                            items: vec![embed(
                                "BankUnit",
                                vec![
                                    named(
                                        "bankPath",
                                        Value::List {
                                            kind: 0x80,
                                            element: 16,
                                            items: vec![text(BANK)],
                                        },
                                    ),
                                    named(
                                        "events",
                                        Value::List {
                                            kind: 0x80,
                                            element: 16,
                                            items: vec![text("Play_sfx_Zed_Skin07_Q")],
                                        },
                                    ),
                                ],
                            )],
                        },
                    )],
                ),
            ),
            named(
                "skinUpgradeData",
                embed(
                    "SkinUpgradeData",
                    vec![named(
                        "mGearSkinUpgrades",
                        Value::List {
                            kind: 0x80,
                            element: LINK,
                            items: gears.iter().map(|g| link(g)).collect(),
                        },
                    )],
                ),
            ),
        ],
    );
    let gear = |key: &str, mesh: &str, skeleton: &str, texture: u64| {
        entry(
            "GearSkinUpgrade",
            key,
            vec![named(
                "mGearData",
                embed(
                    "GearData",
                    vec![named("skinMeshProperties", model(mesh, skeleton, texture))],
                ),
            )],
        )
    };
    serialize_prop_file(&PropFile {
        version: 3,
        links: vec![format!("DATA/Characters/Zed/Animations/Skin{SKIN}.bin")],
        entries: vec![
            skin,
            gear(&gears[0], "ASSETS/Zed/Light.skn", "ASSETS/Zed/Light.skl", 1),
            gear(
                &gears[1],
                "ASSETS/Characters/ZedStorm/Storm.skn",
                "ASSETS/Characters/ZedStorm/Storm.skl",
                0x5707,
            ),
            entry(
                "ContextualActionData",
                "Characters/Zed/CAC/Skin7",
                vec![named(
                    "mSituations",
                    Value::Map {
                        key: crate::gear_toggle::FIELD_HASH,
                        value: 0x83,
                        entries: vec![(
                            hash_value(h(TRANSFORM)),
                            embed("ContextualSituation", Vec::new()),
                        )],
                    },
                )],
            ),
        ],
    })
    .expect("skin")
}

fn transform_clip(mut clip: Value) -> Value {
    let sound = pointer(
        "SoundEventData",
        vec![named(
            "mSoundName",
            text("Play_sfx_Zed_Transform_Storm_buffactivate"),
        )],
    );
    if let Some(fields) = clip.fields_mut() {
        fields.push(named(
            "mEventDataMap",
            Value::Map {
                key: crate::gear_toggle::FIELD_HASH,
                value: 0x82,
                entries: vec![(hash_value(1), sound)],
            },
        ));
    }
    clip
}

fn gear_bank() -> Vec<u8> {
    let gear = |form: usize| crate::sound_bank::wwise_id(&format!("gear_{form}"));
    let mut container = vec![0; 12];
    container.push(0);
    container.extend_from_slice(&0xA2CCu32.to_le_bytes());
    container.extend_from_slice(&gear(0).to_le_bytes());
    container.extend_from_slice(&gear(1).to_le_bytes());
    crate::audio::sound_bank::tests::bank(&[(6, 1, container)])
}

fn maskless_graph() -> Vec<u8> {
    let map = |value: u8, entries: Vec<(&str, Value)>| Value::Map {
        key: crate::gear_toggle::FIELD_HASH,
        value,
        entries: entries
            .into_iter()
            .map(|(k, v)| (hash_value(h(k)), v))
            .collect(),
    };
    let idle = pointer(
        "AtomicClipData",
        vec![
            named("mTrackDataName", hash_value(h("Default"))),
            named(
                "mAnimationResourceData",
                pointer("AnimationResourceData", Vec::new()),
            ),
        ],
    );
    serialize_prop_file(&PropFile {
        version: 3,
        links: Vec::new(),
        entries: vec![entry(
            "AnimationGraphData",
            &format!("Characters/Zed/Animations/Skin{SKIN}"),
            vec![
                named(
                    "mClipDataMap",
                    map(
                        0x82,
                        vec![
                            ("Idle1", idle.clone()),
                            ("Run", idle.clone()),
                            (TRANSFORM, transform_clip(idle)),
                        ],
                    ),
                ),
                named(
                    "mTrackDataMap",
                    map(0x83, vec![("Default", embed("TrackData", Vec::new()))]),
                ),
            ],
        )],
    })
    .expect("graph")
}

fn models() -> Vec<(u64, Vec<u8>)> {
    let light = skeleton(
        vec![
            joint("Root", -1, [0.0; 3]),
            joint("Spine", 0, [0.0, 9.0, 0.0]),
            joint("Arm", 1, [4.0, 0.0, 0.0]),
        ],
        vec![1, 2],
    );
    let mut storm = light.clone();
    storm.joints[2].inverse_bind.rotation = [0.0, 0.0, 0.452, 0.892];
    storm.influences = vec![2];
    let write_skl = |s| crate::skeleton::write(&s).expect("skl");
    let write_skn = |m| crate::skinned_mesh::write(&m).expect("skn");
    vec![
        (wad_path_hash("assets/zed/light.skl"), write_skl(light)),
        (
            wad_path_hash("assets/characters/zedstorm/storm.skl"),
            write_skl(storm),
        ),
        (
            wad_path_hash("assets/zed/light.skn"),
            write_skn(mesh(&[("Light_Body", 3), ("Light_Staff", 1)], true, 1)),
        ),
        (
            wad_path_hash("assets/characters/zedstorm/storm.skn"),
            write_skn(mesh(&[("Storm_Body", 2)], false, 0)),
        ),
        (wad_path_hash(&BANK.to_ascii_lowercase()), gear_bank()),
    ]
}

pub(crate) fn elemental_build(name: &str) -> (PathBuf, String) {
    let mut entries = vec![
        (wad_path_hash(&skin_bin("zed", SKIN)), elementalist_skin()),
        (wad_path_hash(&animation_bin("zed", SKIN)), maskless_graph()),
    ];
    entries.extend(models());
    let game = standard_game(name, &entries);
    let mods_dir = game.join("mods");
    let folder = StandardChampion::open(&game, "Zed")
        .expect("open")
        .build_mod(SKIN, None, &mods_dir)
        .expect("build");
    (mods_dir, folder)
}

#[test]
fn forms_with_their_own_model_cycle_on_one_merged_model_that_only_this_skin_uses() {
    let (mods_dir, folder) = elemental_build("own_models");
    let root = mods_dir.join(&folder).join("WAD").join("Zed.wad.client");

    let skl = std::fs::read(root.join("assets/zed/light_dekanforms.skl")).expect("merged skeleton");
    let skl = crate::skeleton::parse(&skl).expect("skeleton");
    let names: Vec<&str> = skl.joints.iter().map(|j| j.name.as_str()).collect();
    assert_eq!(names, ["Root", "Spine", "Arm", "DekanForm1_Arm"]);
    let skn = std::fs::read(root.join("assets/zed/light_dekanforms.skn")).expect("merged mesh");
    let skn = crate::skinned_mesh::parse(&skn).expect("mesh");
    let parts: Vec<&str> = skn.submeshes.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(parts, ["Light_Body", "Light_Staff", "Storm_Body"]);
    assert!(
        !root.join("assets/zed/light.skn").exists(),
        "the game's own model is never replaced, so a player who owns the skin keeps it"
    );

    let graph = std::fs::read(root.join(format!("data/characters/zed/animations/skin{SKIN}.bin")))
        .expect("form graph");
    let graph = parse_prop_file(&graph).expect("graph");
    let fields = tree::parse_fields(&graph.entries[0].body).expect("fields");
    let masks = tree::field(&fields, h("mMaskDataMap")).expect("a mask map is added");
    let Value::Map { entries, .. } = masks else {
        panic!("masks are a map");
    };
    let weights = entries[0]
        .1
        .fields()
        .and_then(|f| tree::field(f, h("mWeightList")))
        .and_then(Value::items);
    assert_eq!(
        weights.map(<[Value]>::len),
        Some(4),
        "one weight per merged joint"
    );

    let skin0 = generated_skin0(&mods_dir, &folder, "zed").expect("skin0");
    let skin0 = serialize_prop_file(&skin0).expect("bytes");
    assert_eq!(
        crate::form_marker::mesh_text(&skin0, "simpleSkin")
            .expect("mesh")
            .as_deref(),
        Some("ASSETS/Zed/Light_DekanForms.skn")
    );
    assert_eq!(
        crate::form_marker::mesh_text(&skin0, "skeleton")
            .expect("skeleton")
            .as_deref(),
        Some("ASSETS/Zed/Light_DekanForms.skl")
    );
    let hidden = crate::form_marker::mesh_text(&skin0, "initialSubmeshToHide")
        .expect("hidden")
        .unwrap_or_default();
    assert!(
        hidden.split_whitespace().any(|p| p == "Storm_Body"),
        "{hidden}"
    );
    let parsed = parse_prop_file(&skin0).expect("skin0");
    let skin = parsed
        .entries
        .iter()
        .find(|e| e.class_hash == SKIN_DATA_CLASS)
        .expect("skin");
    let skin = tree::parse_fields(&skin.body).expect("skin fields");
    let overrides = tree::field(&skin, h("skinMeshProperties"))
        .and_then(Value::fields)
        .and_then(|m| tree::field(m, h("materialOverride")))
        .and_then(Value::items)
        .expect("per-part textures");
    let storm = overrides
        .iter()
        .filter_map(Value::fields)
        .find(|f| tree::field(f, h("submesh")) == Some(&text("Storm_Body")))
        .expect("the storm body keeps its own texture");
    assert_eq!(tree::field(storm, h("texture")), Some(&file(0x5707)));
}
