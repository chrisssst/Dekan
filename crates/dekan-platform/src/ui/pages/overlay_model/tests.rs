use super::*;
use crate::i18n::Language;
use dekan_core::mods::{ModPackage, ModSource};
use dekan_core::overlay::{CatalogChroma, ModsPanel};

fn chroma(id: u32, name: &str) -> CatalogChroma {
    CatalogChroma {
        id,
        name: name.into(),
        color: Some("#ff8800".into()),
        form: false,
        preview_path: None,
        has_preview: false,
    }
}

fn skin(id: u32, name: &str, chromas: Vec<CatalogChroma>) -> CatalogSkin {
    CatalogSkin {
        id,
        name: name.into(),
        name_unknown: false,
        chromas,
        tile: None,
    }
}

fn catalog(skins: Vec<CatalogSkin>) -> Catalog {
    Catalog {
        champion_id: 103,
        champion_name: "Ahri".into(),
        alias: Some("Ahri".into()),
        skins,
        locale: Some("pt_BR".into()),
        quote: None,
        mods: ModsPanel::default(),
        notice: None,
        classic: false,
        lobby: Vec::new(),
    }
}

fn entry(id: &str, name: &str, category: ModCategory) -> ModEntry {
    ModEntry {
        id: id.into(),
        name: name.into(),
        category,
        source: ModSource::Dekan,
        path: std::path::PathBuf::new(),
        package: ModPackage::Directory,
        description: None,
    }
}

fn ahri() -> Catalog {
    catalog(vec![
        skin(103_000, "Ahri", vec![]),
        skin(103_001, "Ahri Dinastia", vec![]),
        skin(
            103_027,
            "Ahri Arcana",
            vec![chroma(103_028, "Pérola Solar"), chroma(103_029, "Rubi")],
        ),
        skin(103_042, "Ahri Guardiã Estelar", vec![]),
    ])
}

#[test]
fn folding_ignores_case_and_every_accent_a_skin_name_uses() {
    assert_eq!(
        fold("Ação ÉBANO Ñandú Ölçü Łódź"),
        "acao ebano nandu olcu lodz"
    );
    assert_eq!(fold("Guardiã Estelar"), fold("GUARDIA ESTELAR"));
    assert_eq!(fold("K'Sante 2077"), "k'sante 2077");
}

#[test]
fn search_matches_name_without_accents_chroma_names_and_ids() {
    let ahri = ahri();
    let names =
        |search: &str| -> Vec<u32> { visible_skins(&ahri, search).iter().map(|s| s.id).collect() };
    assert_eq!(names(""), vec![103_000, 103_001, 103_027, 103_042]);
    assert_eq!(names("  guardia "), vec![103_042]);
    assert_eq!(
        names("PEROLA"),
        vec![103_027],
        "a chroma name finds its skin"
    );
    assert_eq!(names("103001"), vec![103_001], "an id finds its skin");
    assert!(names("zed").is_empty());
}

#[test]
fn cards_are_split_into_rows_of_the_column_count_and_never_zero() {
    let items: Vec<u32> = (0..7).collect();
    assert_eq!(
        chunk(&items, 3),
        vec![vec![0, 1, 2], vec![3, 4, 5], vec![6]]
    );
    assert_eq!(chunk(&items, 0).len(), 7, "zero columns falls back to one");
    assert!(chunk::<u32>(&[], 2).is_empty());
}

#[test]
fn clicking_the_selected_entry_clears_it_and_any_other_selects_it() {
    assert_eq!(choose(None, 7), (Some(7), OverlayCommand::Select { id: 7 }));
    assert_eq!(choose(Some(7), 7), (None, OverlayCommand::Clear));
    assert_eq!(
        choose(Some(7), 8),
        (Some(8), OverlayCommand::Select { id: 8 })
    );
}

#[test]
fn a_chroma_selection_highlights_its_parent_skin() {
    let ahri = ahri();
    assert_eq!(parent_skin(&ahri, 103_029), Some(103_027));
    assert_eq!(parent_skin(&ahri, 103_001), Some(103_001));
    assert_eq!(parent_skin(&ahri, 999), None);
}

#[test]
fn empty_state_explains_search_missing_library_or_waiting() {
    let text = Language::English.text();
    let mut empty = catalog(vec![]);
    assert_eq!(
        empty_texts(&empty, " vayne ", text),
        (
            "No skin found".to_owned(),
            "No skin in the library matches “vayne”.".to_owned()
        )
    );
    assert_eq!(
        empty_texts(&empty, "", text).0,
        "No skin in the library for Ahri"
    );
    empty.classic = true;
    assert_eq!(
        empty_texts(&empty, "", text).0,
        "No skin in the library for Ahri · Classic Rift"
    );
    empty.champion_name.clear();
    assert_eq!(empty_texts(&empty, "", text).0, text.overlay_waiting_big);
}

#[test]
fn footer_counts_entries_or_shows_the_quote_only_in_portuguese() {
    let text = Language::English.text();
    let mut ahri = ahri();
    assert_eq!(footer(&ahri, false, text), ("6 skins".to_owned(), false));
    ahri.skins.truncate(1);
    assert_eq!(footer(&ahri, false, text), ("1 skin".to_owned(), false));
    ahri.quote = Some("Eu sempre atiro primeiro.".into());
    assert_eq!(
        footer(&ahri, true, text),
        ("“Eu sempre atiro primeiro.”".to_owned(), true)
    );
    assert!(
        !footer(&ahri, false, text).1,
        "no guessed translation of the quote"
    );
}

#[test]
fn champion_label_and_initials_handle_missing_and_unicode_names() {
    let text = Language::Turkish.text();
    let mut ahri = ahri();
    assert_eq!(champion_label(&ahri, text), "Ahri");
    ahri.champion_name.clear();
    assert_eq!(champion_label(&ahri, text), text.overlay_no_champion);
    assert_eq!(initial("  élise"), "É");
    assert_eq!(initial(""), "?");
}

fn library() -> ModCatalog {
    ModCatalog {
        skin: vec![entry("dekan:skin/a", "Ahri Neon", ModCategory::Skin)],
        map: vec![
            entry("dekan:map/w", "Winter Rift", ModCategory::Map),
            entry("dekan:map/n", "Night Rift", ModCategory::Map),
        ],
        font: vec![],
        announcer: vec![],
        others: vec![
            entry("dekan:vfx/1", "Golden Wards", ModCategory::Vfx),
            entry("dekan:ui/1", "Clean HUD", ModCategory::Ui),
        ],
    }
}

#[test]
fn mod_lines_list_single_slots_with_a_none_row_then_others_by_category() {
    let text = Language::English.text();
    let selection = ModSelectionView {
        map: Some("dekan:map/n".into()),
        others: vec!["dekan:vfx/1".into()],
        ..Default::default()
    };
    let lines = mod_lines(&library(), &selection, "", text);
    let shape: Vec<(ModLineKind, String, bool)> = lines
        .iter()
        .map(|line| (line.kind, line.text.to_string(), line.selected))
        .collect();
    assert_eq!(
        shape,
        vec![
            (ModLineKind::Section, "Skin mod".into(), false),
            (ModLineKind::Entry, "None".into(), true),
            (ModLineKind::Entry, "Ahri Neon".into(), false),
            (ModLineKind::Section, "Map".into(), false),
            (ModLineKind::Entry, "None".into(), false),
            (ModLineKind::Entry, "Winter Rift".into(), false),
            (ModLineKind::Entry, "Night Rift".into(), true),
            (ModLineKind::Section, "Others".into(), false),
            (ModLineKind::Group, "Interface".into(), false),
            (ModLineKind::Entry, "Clean HUD".into(), false),
            (ModLineKind::Group, "Visual effects".into(), false),
            (ModLineKind::Entry, "Golden Wards".into(), true),
        ]
    );
    assert!(lines.iter().filter(|l| l.slot == "map").all(|l| l.radio));
    assert!(
        lines
            .iter()
            .filter(|l| l.slot == "others")
            .all(|l| !l.radio)
    );
}

#[test]
fn mod_search_filters_without_accents_and_explains_no_match() {
    let text = Language::English.text();
    let lines = mod_lines(&library(), &ModSelectionView::default(), "WÍNTER", text);
    let entries: Vec<String> = lines
        .iter()
        .filter(|l| l.kind == ModLineKind::Entry && !l.none)
        .map(|l| l.text.to_string())
        .collect();
    assert_eq!(entries, vec!["Winter Rift"]);

    let none = mod_lines(&library(), &ModSelectionView::default(), "zzz", text);
    assert_eq!(none.len(), 1);
    assert_eq!(none[0].kind, ModLineKind::Message);
    assert_eq!(none[0].text, "No mod matches “zzz”.");

    let empty = mod_lines(
        &ModCatalog::default(),
        &ModSelectionView::default(),
        "",
        text,
    );
    assert_eq!(empty.len(), 1);
    assert_eq!(empty[0].kind, ModLineKind::Empty);
}

#[test]
fn toggling_mods_sets_single_slots_and_flips_others() {
    let base = ModSelectionView::default();
    let map = toggle_mod(&base, "map", "dekan:map/w").expect("map slot");
    assert_eq!(map.map.as_deref(), Some("dekan:map/w"));
    let cleared = toggle_mod(&map, "map", "").expect("none row");
    assert_eq!(cleared.map, None);

    let one = toggle_mod(&base, "others", "dekan:ui/1").expect("others");
    let two = toggle_mod(&one, "others", "dekan:vfx/1").expect("others");
    assert_eq!(two.others, vec!["dekan:ui/1", "dekan:vfx/1"]);
    let back = toggle_mod(&two, "others", "dekan:ui/1").expect("others");
    assert_eq!(back.others, vec!["dekan:vfx/1"]);

    assert_eq!(
        toggle_mod(&base, "others", ""),
        None,
        "others has no none row"
    );
    assert_eq!(
        toggle_mod(&base, "C:/Windows", "x"),
        None,
        "unknown slots are refused"
    );
    assert_eq!(selected_count(&two), 2);
    assert_eq!(selected_count(&map), 1);
}

#[test]
fn import_offers_every_category_once_with_a_label_in_every_language() {
    let categories = import_categories();
    let mut sorted = categories.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), ModCategory::ALL.len());
    for language in [Language::Turkish, Language::English] {
        for category in &categories {
            assert!(
                !category_label(*category, language.text()).is_empty(),
                "{language:?} {category:?}"
            );
        }
    }
}

#[test]
fn the_tools_notice_appears_only_when_flagged() {
    let text = Language::English.text();
    let mut ahri = ahri();
    assert_eq!(notice(&ahri, text), "");
    ahri.notice = Some(CatalogNotice::ToolsMissing);
    assert_eq!(notice(&ahri, text), text.overlay_tools_missing);
}
