use super::*;
use crate::i18n::Language;
use dekan_core::mods::{ModCatalog, ModCategory, ModEntry, ModPackage, ModSource};
use dekan_core::overlay::{CatalogChroma, CatalogSkin};
use i_slint_backend_testing::ElementHandle;
use slint::Model;

fn client_rect() -> WindowRect {
    WindowRect {
        left: 0,
        top: 0,
        right: 1600,
        bottom: 900,
    }
}

#[test]
fn test_hidden_client_hides_the_overlay() {
    assert!(decide_placement(ClientWindowState::Hidden, true, None).is_none());
    assert!(decide_placement(ClientWindowState::Absent, true, None).is_none());
}

#[test]
fn test_overlay_is_not_shown_when_not_wanted() {
    assert!(
        decide_placement(ClientWindowState::Visible(client_rect()), false, None).is_none(),
        "outside champ select the overlay must stay hidden even with the client on screen"
    );
}

#[test]
fn test_visible_client_places_the_overlay_alongside_it() {
    let placement = decide_placement(ClientWindowState::Visible(client_rect()), true, None)
        .expect("overlay should be placed");
    assert_eq!(placement.width(), OVERLAY_WIDTH);
    assert!(placement.left >= client_rect().right);
}

#[test]
fn chroma_colors_parse_only_full_hex_and_fall_back_otherwise() {
    assert_eq!(
        parse_color(Some("#ff8800")),
        slint::Color::from_rgb_u8(0xff, 0x88, 0x00)
    );
    let fallback = slint::Color::from_rgb_u8(0x3a, 0x4a, 0x5a);
    for bad in [
        None,
        Some(""),
        Some("ff8800"),
        Some("#f80"),
        Some("#gg0000"),
        Some("#ff88001"),
    ] {
        assert_eq!(parse_color(bad), fallback, "{bad:?}");
    }
}

fn catalog() -> Catalog {
    let chroma = |id: u32, name: &str| CatalogChroma {
        id,
        name: name.into(),
        color: Some("#40c0ff".into()),
        form: false,
        preview_path: None,
        has_preview: false,
    };
    let skin = |id: u32, name: &str, chromas: Vec<CatalogChroma>| CatalogSkin {
        id,
        name: name.into(),
        name_unknown: false,
        chromas,
        tile: None,
    };
    let mods = ModsPanel {
        available: ModCatalog {
            map: vec![ModEntry {
                id: "dekan:map/w".into(),
                name: "Winter Rift".into(),
                category: ModCategory::Map,
                source: ModSource::Dekan,
                path: std::path::PathBuf::new(),
                package: ModPackage::Directory,
                description: None,
            }],
            ..ModCatalog::default()
        },
        ..ModsPanel::default()
    };
    Catalog {
        champion_id: 238,
        champion_name: "Zed".into(),
        alias: Some("Zed".into()),
        skins: vec![
            skin(238_000, "Zed", vec![]),
            skin(
                238_001,
                "Zed Choque",
                vec![chroma(238_070, "Esmeralda"), chroma(238_071, "Obsidiana")],
            ),
            skin(238_010, "Zed Projeto", vec![]),
        ],
        locale: Some("en_US".into()),
        quote: None,
        mods,
        notice: None,
        classic: false,
        lobby: Vec::new(),
    }
}

fn open_overlay() -> (views::OverlayWindow, UnboundedReceiver<OverlayCommand>) {
    i_slint_backend_testing::init_no_event_loop();
    let (tx, rx) = unbounded_channel();
    let view = create(tx).expect("overlay window");
    view.window()
        .set_size(slint::LogicalSize::new(620.0, 900.0));
    with_overlay(|overlay| overlay.set_catalog(catalog()));
    (view, rx)
}

fn card_ids(view: &views::OverlayWindow) -> Vec<i32> {
    view.get_rows()
        .iter()
        .flat_map(|row| row.cards.iter().map(|card| card.id).collect::<Vec<_>>())
        .collect()
}

#[test]
fn the_real_overlay_lists_the_catalog_and_a_gem_click_selects_through_the_command_channel() {
    let (view, mut commands) = open_overlay();
    assert_eq!(card_ids(&view), vec![238_000, 238_001, 238_010]);
    assert_eq!(view.get_champion(), "Zed");
    assert_eq!(view.get_footer_right(), "5 skins");

    let gem = ElementHandle::find_by_accessible_label(&view, "Obsidiana")
        .next()
        .expect("the chroma gem is on screen");
    gem.invoke_accessible_default_action();
    assert_eq!(
        commands.try_recv().ok(),
        Some(OverlayCommand::Select { id: 238_071 })
    );
    assert_eq!(view.get_selected_id(), 238_071);
    assert_eq!(
        view.get_selected_skin(),
        238_001,
        "the gem lights its parent card"
    );

    gem.invoke_accessible_default_action();
    assert_eq!(commands.try_recv().ok(), Some(OverlayCommand::Clear));
    assert_eq!(view.get_selected_id(), -1);
}

#[test]
fn typing_in_search_filters_the_cards_without_accents() {
    let (view, _commands) = open_overlay();
    view.invoke_search_edited("ESMERÁLDA".into());
    assert_eq!(card_ids(&view), vec![238_001]);
    view.invoke_search_edited("nothing".into());
    assert!(card_ids(&view).is_empty());
    assert_eq!(view.get_empty_big(), "No skin found");
}

#[test]
fn the_mods_tab_selects_a_map_and_reports_it() {
    let (view, mut commands) = open_overlay();
    view.invoke_show_tab(true);
    assert!(view.get_mods_tab());
    let row = ElementHandle::find_by_accessible_label(&view, "Winter Rift")
        .next()
        .expect("the map mod is listed");
    row.invoke_accessible_default_action();
    let expected = ModSelectionView {
        map: Some("dekan:map/w".into()),
        ..ModSelectionView::default()
    };
    assert_eq!(
        commands.try_recv().ok(),
        Some(OverlayCommand::SetMods {
            selection: expected
        })
    );
    assert_eq!(view.get_mods_count(), "1");
    let row = ElementHandle::find_by_accessible_label(&view, "Winter Rift")
        .next()
        .expect("the map mod is still listed");
    assert_eq!(row.accessible_checked(), Some(true));
}

#[test]
fn columns_regroup_cards_and_a_new_catalog_resets_selection_and_search() {
    let (view, _commands) = open_overlay();
    view.invoke_columns_changed(2);
    let sizes: Vec<usize> = view
        .get_rows()
        .iter()
        .map(|row| row.cards.row_count())
        .collect();
    assert_eq!(sizes, vec![2, 1]);

    view.invoke_choose(238_010);
    view.invoke_search_edited("projeto".into());
    with_overlay(|overlay| overlay.set_catalog(catalog()));
    assert_eq!(view.get_selected_id(), -1);
    assert_eq!(view.get_search(), "");
    assert_eq!(card_ids(&view).len(), 3);
}

#[test]
fn the_historic_origin_tags_the_selected_card() {
    let (view, _commands) = open_overlay();
    with_overlay(|overlay| {
        overlay.selected = Some(238_070);
        overlay.origin = Some(SelectionOrigin::Historic);
        overlay.render_selection();
    });
    assert_eq!(view.get_origin(), views::SelectionOrigin::Historic);
    assert_eq!(view.get_selected_skin(), 238_001);
}

fn press(view: &views::OverlayWindow, label: &str) {
    ElementHandle::find_by_accessible_label(view, label)
        .next()
        .unwrap_or_else(|| panic!("nothing labelled {label:?} is on screen"))
        .invoke_accessible_default_action();
}

fn drain(commands: &mut UnboundedReceiver<OverlayCommand>) -> Vec<OverlayCommand> {
    let mut seen = Vec::new();
    while let Ok(command) = commands.try_recv() {
        seen.push(command);
    }
    seen
}

#[test]
fn every_control_of_the_skins_tab_sends_what_it_says() {
    let (view, mut commands) = open_overlay();
    let text = Language::English.text();

    press(&view, "Zed Projeto");
    assert_eq!(view.get_selected_skin(), 238_010);
    press(&view, "Zed Projeto");
    press(&view, text.overlay_dice);
    press(&view, "Esmeralda");
    assert_eq!(
        drain(&mut commands),
        vec![
            OverlayCommand::Select { id: 238_010 },
            OverlayCommand::Clear,
            OverlayCommand::Random,
            OverlayCommand::Select { id: 238_070 },
        ]
    );
    assert_eq!(
        ElementHandle::find_by_accessible_label(&view, "Zed Choque")
            .next()
            .and_then(|card| card.accessible_checked()),
        Some(true),
        "the card of the chosen chroma reads as selected"
    );
}

#[test]
fn tabs_switch_views_clear_the_search_and_the_mods_actions_send_their_commands() {
    let (view, mut commands) = open_overlay();
    let text = Language::English.text();
    view.set_search("zed".into());
    view.invoke_search_edited("zed".into());

    press(&view, text.overlay_tab_mods);
    assert!(view.get_mods_tab());
    assert_eq!(
        view.get_search(),
        "",
        "switching tabs starts a fresh search"
    );
    assert!(
        card_ids(&view).len() == 3,
        "the skins list keeps its catalog behind the tab"
    );

    view.set_import_index(2);
    press(&view, text.overlay_import_mod);
    press(&view, text.overlay_open_folder);
    assert_eq!(
        drain(&mut commands),
        vec![
            OverlayCommand::ImportMod {
                category: ModCategory::Font
            },
            OverlayCommand::OpenModsFolder,
        ]
    );
    assert_eq!(
        view.get_import_categories().row_count(),
        ModCategory::ALL.len()
    );

    press(&view, text.overlay_tab_skins);
    assert!(!view.get_mods_tab());
}

#[test]
fn hovering_a_gem_asks_for_its_preview_once_and_shows_it_when_delivered() {
    let (view, mut commands) = open_overlay();
    let gem = ChromaGem {
        id: 238_071,
        name: "Obsidiana".into(),
        color: slint::Color::default(),
        form: false,
        has_preview: true,
    };
    view.invoke_gem_hovered(gem.clone(), 120.0, 200.0, true);
    assert!(view.get_preview_visible());
    assert!(view.get_preview_loading());
    assert_eq!(view.get_preview_name(), "Obsidiana");
    assert_eq!(
        drain(&mut commands),
        vec![OverlayCommand::ChromaPreview { id: 238_071 }]
    );

    let png =
        std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/assets/logo.png")).expect("png");
    with_overlay(|overlay| overlay.deliver_preview(238_071, &png));
    assert!(!view.get_preview_loading());
    assert_eq!(view.get_preview_image().size().width, 72);

    view.invoke_gem_hovered(gem.clone(), 120.0, 200.0, false);
    assert!(!view.get_preview_visible());
    view.invoke_gem_hovered(gem, 120.0, 200.0, true);
    assert!(
        drain(&mut commands).is_empty(),
        "a cached preview is not asked again"
    );
    assert!(!view.get_preview_loading());

    view.invoke_scrolled();
    assert!(!view.get_preview_visible(), "scrolling hides the preview");
}

#[test]
fn a_gem_without_a_preview_never_asks_for_one() {
    let (view, mut commands) = open_overlay();
    let gem = ChromaGem {
        id: 238_070,
        name: "Esmeralda".into(),
        color: slint::Color::default(),
        form: false,
        has_preview: false,
    };
    view.invoke_gem_hovered(gem, 10.0, 10.0, true);
    assert!(!view.get_preview_visible());
    assert!(drain(&mut commands).is_empty());
}

fn bounds(element: &ElementHandle) -> (f32, f32, f32, f32) {
    let at = element.absolute_position();
    let size = element.size();
    (at.x, at.y, at.x + size.width, at.y + size.height)
}

#[test]
fn every_gem_stays_inside_its_card_at_any_window_width() {
    let (view, _commands) = open_overlay();
    let many: Vec<CatalogChroma> = (0..23)
        .map(|i| CatalogChroma {
            id: 238_100 + i,
            name: format!("Gem {i}"),
            color: Some("#3050c0".into()),
            form: false,
            preview_path: None,
            has_preview: false,
        })
        .collect();
    let mut wide = catalog();
    wide.skins[1].chromas = many;
    with_overlay(|overlay| overlay.set_catalog(wide));

    for width in [320.0, 360.0, 480.0, 620.0, 900.0] {
        view.window()
            .set_size(slint::LogicalSize::new(width, 1400.0));
        view.invoke_columns_changed(view.get_columns());
        let card = ElementHandle::find_by_accessible_label(&view, "Zed Choque")
            .next()
            .expect("the card with chromas");
        let (left, top, right, bottom) = bounds(&card);
        for i in 0..23 {
            let gem = ElementHandle::find_by_accessible_label(&view, &format!("Gem {i}"))
                .next()
                .unwrap_or_else(|| panic!("gem {i} at width {width}"));
            let (gl, gt, gr, gb) = bounds(&gem);
            assert!(
                gl >= left && gr <= right && gt >= top && gb <= bottom,
                "gem {i} ({gl},{gt})-({gr},{gb}) escapes its card ({left},{top})-({right},{bottom}) at width {width}"
            );
        }
    }
}

#[test]
fn minimizing_folds_the_window_to_its_header_and_restoring_brings_the_size_back() {
    let (view, _commands) = open_overlay();
    let text = Language::English.text();
    let before = view.window().size();

    press(&view, text.overlay_minimize);
    assert!(view.get_collapsed());
    let folded = view
        .window()
        .size()
        .to_logical(view.window().scale_factor());
    assert!(
        folded.height <= view.get_header_height() + 1.0,
        "folded to {folded:?}, header is {}",
        view.get_header_height()
    );
    assert!(
        ElementHandle::find_by_accessible_label(&view, text.overlay_hide)
            .next()
            .is_some(),
        "hide stays reachable"
    );

    press(&view, text.overlay_restore);
    assert!(!view.get_collapsed());
    assert_eq!(view.window().size(), before);
}

#[test]
fn a_lobby_with_two_champions_offers_a_tab_for_each_and_switching_asks_for_it() {
    let (view, mut commands) = open_overlay();
    let mut lobby = catalog();
    lobby.lobby = vec![
        dekan_core::overlay::LobbyChampion {
            id: 238,
            name: "Zed".into(),
        },
        dekan_core::overlay::LobbyChampion {
            id: 103,
            name: "Ahri".into(),
        },
    ];
    lobby.notice = Some(dekan_core::overlay::CatalogNotice::LobbyChampions);
    with_overlay(|overlay| overlay.set_catalog(lobby));

    let tabs: Vec<(i32, bool)> = view
        .get_lobby_champions()
        .iter()
        .map(|choice| (choice.id, choice.active))
        .collect();
    assert_eq!(tabs, vec![(238, true), (103, false)]);
    assert_eq!(
        view.get_notice(),
        Language::English.text().overlay_lobby_champions
    );

    drain(&mut commands);
    press(&view, "Ahri");
    assert_eq!(
        drain(&mut commands),
        vec![OverlayCommand::FocusChampion { id: 103 }]
    );
}

#[test]
fn a_lobby_without_champions_says_where_to_pick_them() {
    let (view, _commands) = open_overlay();
    with_overlay(|overlay| {
        overlay.set_catalog(Catalog {
            locale: Some("en_US".into()),
            notice: Some(dekan_core::overlay::CatalogNotice::LobbyWaiting),
            ..Catalog::default()
        })
    });
    let text = Language::English.text();
    assert_eq!(view.get_empty_big(), text.overlay_lobby_waiting_big);
    assert!(view.get_lobby_champions().row_count() == 0);
    assert_eq!(view.get_notice(), "");
}

#[test]
fn the_pin_lights_for_the_champions_preset_and_profiles_list_the_default_first() {
    let (view, mut commands) = open_overlay();
    let controller_presets = PresetsView {
        profiles: vec![String::new(), "Ranked".into()],
        active: 1,
        preset_entry: Some(238_071),
    };
    with_overlay(|overlay| {
        overlay.presets = controller_presets;
        overlay.render_presets();
        overlay.render_selection();
    });
    let text = Language::English.text();
    let names: Vec<String> = view.get_profiles().iter().map(|n| n.to_string()).collect();
    assert_eq!(
        names,
        vec![text.overlay_profile_default.to_owned(), "Ranked".to_owned()]
    );
    assert_eq!(view.get_profile_index(), 1);
    assert!(view.get_can_delete_profile());
    assert!(!view.get_can_pin(), "nothing chosen yet");

    press(&view, "Obsidiana");
    assert!(view.get_can_pin());
    assert!(
        view.get_pinned(),
        "the chosen chroma is this profile's preset"
    );
    drain(&mut commands);

    press(&view, text.overlay_unpin);
    press(&view, text.overlay_profile_new);
    press(&view, text.overlay_profile_delete);
    assert_eq!(
        drain(&mut commands),
        vec![
            OverlayCommand::TogglePreset,
            OverlayCommand::NewProfile,
            OverlayCommand::DeleteProfile
        ]
    );

    press(&view, "Esmeralda");
    assert!(!view.get_pinned());
}
