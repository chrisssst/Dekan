use super::*;
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

pub(super) fn catalog() -> Catalog {
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

pub(super) fn open_overlay() -> (views::OverlayWindow, UnboundedReceiver<OverlayCommand>) {
    i_slint_backend_testing::init_no_event_loop();
    let (tx, rx) = unbounded_channel();
    let view = create(tx).expect("overlay window");
    view.window()
        .set_size(slint::LogicalSize::new(620.0, 900.0));
    with_overlay(|overlay| overlay.set_catalog(catalog()));
    (view, rx)
}

pub(super) fn card_ids(view: &views::OverlayWindow) -> Vec<i32> {
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

#[test]
fn the_party_line_shows_the_room_and_teammate_skins_and_goes_away_outside_a_party() {
    let (view, _commands) = open_overlay();
    assert_eq!(view.get_party_line(), "");

    with_overlay(|overlay| {
        overlay.party = Some(dekan_core::party::PartySummary {
            members: 3,
            teammate_skins: 2,
        });
        overlay.render_party();
    });
    assert_eq!(
        view.get_party_line(),
        "party: 3 in the room, 2 teammate skin(s)"
    );

    with_overlay(|overlay| {
        overlay.party = None;
        overlay.render_party();
    });
    assert_eq!(view.get_party_line(), "");
}

#[test]
fn overlay_starts_with_the_saved_theme_and_tracks_changes_without_selection_commands() {
    let (view, mut commands) = open_overlay();
    assert_eq!(
        view.global::<views::Theme>().get_mode(),
        crate::preferences::theme_mode()
    );
    let initial_cards = card_ids(&view);
    for mode in [0, 2, 1, 0] {
        super::super::themes::apply(mode);
        assert_eq!(view.global::<views::Theme>().get_mode(), mode);
        assert_eq!(card_ids(&view), initial_cards);
        assert!(commands.try_recv().is_err());
    }
}
