use dekan_core::mods::{ModCatalog, ModCategory, ModEntry, ModSelectionView};
use dekan_core::overlay::{Catalog, CatalogNotice, CatalogSkin, OverlayCommand};
use slint::SharedString;

use super::views::{ModLine, ModLineKind};
use crate::i18n::{Text, fill};

pub(crate) const SINGLE_SLOTS: [(&str, ModCategory); 4] = [
    ("skin", ModCategory::Skin),
    ("map", ModCategory::Map),
    ("font", ModCategory::Font),
    ("announcer", ModCategory::Announcer),
];

pub(crate) const OTHER_CATEGORIES: [ModCategory; 6] = [
    ModCategory::Ui,
    ModCategory::Voiceover,
    ModCategory::LoadingScreen,
    ModCategory::Vfx,
    ModCategory::Sfx,
    ModCategory::Other,
];

#[must_use]
pub(crate) fn fold(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .map(strip_accent)
        .collect()
}

fn strip_accent(c: char) -> char {
    match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => 'a',
        'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' => 'c',
        'ď' | 'đ' => 'd',
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => 'e',
        'ĝ' | 'ğ' | 'ġ' | 'ģ' => 'g',
        'ĥ' | 'ħ' => 'h',
        'ì' | 'í' | 'î' | 'ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' | 'ı' => 'i',
        'ĵ' => 'j',
        'ķ' => 'k',
        'ĺ' | 'ļ' | 'ľ' | 'ŀ' | 'ł' => 'l',
        'ñ' | 'ń' | 'ņ' | 'ň' => 'n',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ŏ' | 'ő' => 'o',
        'ŕ' | 'ŗ' | 'ř' => 'r',
        'ś' | 'ŝ' | 'ş' | 'š' => 's',
        'ţ' | 'ť' | 'ŧ' => 't',
        'ù' | 'ú' | 'û' | 'ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => 'u',
        'ŵ' => 'w',
        'ý' | 'ÿ' | 'ŷ' => 'y',
        'ź' | 'ż' | 'ž' => 'z',
        other => other,
    }
}

#[must_use]
pub(crate) fn skin_matches(skin: &CatalogSkin, term: &str) -> bool {
    term.is_empty()
        || fold(&skin.name).contains(term)
        || skin.id.to_string().contains(term)
        || skin
            .chromas
            .iter()
            .any(|chroma| fold(&chroma.name).contains(term))
}

#[must_use]
pub(crate) fn visible_skins<'a>(catalog: &'a Catalog, search: &str) -> Vec<&'a CatalogSkin> {
    let term = fold(search.trim());
    catalog
        .skins
        .iter()
        .filter(|skin| skin_matches(skin, &term))
        .collect()
}

#[must_use]
pub(crate) fn chunk<T: Clone>(items: &[T], columns: usize) -> Vec<Vec<T>> {
    items.chunks(columns.max(1)).map(<[T]>::to_vec).collect()
}

#[must_use]
pub(crate) fn parent_skin(catalog: &Catalog, entry_id: u32) -> Option<u32> {
    catalog
        .resolve_target(entry_id)
        .map(|target| target.skin_id)
}

#[must_use]
pub(crate) fn choose(current: Option<u32>, clicked: u32) -> (Option<u32>, OverlayCommand) {
    if current == Some(clicked) {
        (None, OverlayCommand::Clear)
    } else {
        (Some(clicked), OverlayCommand::Select { id: clicked })
    }
}

#[must_use]
pub(crate) fn champion_label(catalog: &Catalog, text: &Text) -> String {
    match catalog.champion_name.as_str() {
        "" => text.overlay_no_champion.to_owned(),
        name if catalog.classic => format!("{name} {}", text.overlay_classic_suffix),
        name => name.to_owned(),
    }
}

#[must_use]
pub(crate) fn initial(name: &str) -> String {
    name.trim()
        .chars()
        .next()
        .map_or_else(|| "?".to_owned(), |c| c.to_uppercase().collect())
}

#[must_use]
pub(crate) fn empty_texts(catalog: &Catalog, search: &str, text: &Text) -> (String, String) {
    let search = search.trim();
    if !search.is_empty() {
        (
            text.overlay_no_results_big.to_owned(),
            fill(text.overlay_no_results_sub, "term", search),
        )
    } else if catalog.notice == Some(CatalogNotice::LobbyWaiting) {
        (
            text.overlay_lobby_waiting_big.to_owned(),
            text.overlay_lobby_waiting_sub.to_owned(),
        )
    } else if !catalog.champion_name.is_empty() {
        (
            fill(
                text.overlay_no_library_big,
                "champion",
                &champion_label(catalog, text),
            ),
            text.overlay_no_library_sub.to_owned(),
        )
    } else {
        (
            text.overlay_waiting_big.to_owned(),
            text.overlay_waiting_sub.to_owned(),
        )
    }
}

#[must_use]
pub(crate) fn footer(catalog: &Catalog, portuguese: bool, text: &Text) -> (String, bool) {
    match &catalog.quote {
        Some(quote) if portuguese => (format!("“{quote}”"), true),
        _ => match catalog.entry_count() {
            1 => (text.overlay_skin_one.to_owned(), false),
            n => (fill(text.overlay_skin_many, "count", &n.to_string()), false),
        },
    }
}

#[must_use]
pub(crate) fn notice(catalog: &Catalog, text: &Text) -> &'static str {
    match catalog.notice {
        Some(CatalogNotice::ToolsMissing) => text.overlay_tools_missing,
        Some(CatalogNotice::LobbyChampions) => text.overlay_lobby_champions,
        Some(CatalogNotice::LobbyWaiting) | None => "",
    }
}

#[must_use]
pub(crate) fn profile_label(name: &str, text: &Text) -> String {
    if name == dekan_core::presets::DEFAULT_PROFILE {
        text.overlay_profile_default.to_owned()
    } else {
        name.to_owned()
    }
}

#[must_use]
pub(crate) fn lobby_choices(catalog: &Catalog) -> Vec<(u32, String, bool)> {
    if catalog.lobby.len() < 2 {
        return Vec::new();
    }
    catalog
        .lobby
        .iter()
        .map(|champion| {
            (
                champion.id,
                champion.name.clone(),
                champion.id == catalog.champion_id,
            )
        })
        .collect()
}

#[must_use]
pub(crate) fn selected_count(selection: &ModSelectionView) -> usize {
    [
        &selection.skin,
        &selection.map,
        &selection.font,
        &selection.announcer,
    ]
    .into_iter()
    .filter(|slot| slot.is_some())
    .count()
        + selection.others.len()
}

#[must_use]
pub(crate) fn category_label(category: ModCategory, text: &Text) -> &'static str {
    match category {
        ModCategory::Skin => text.overlay_slot_skin,
        ModCategory::Map => text.overlay_slot_map,
        ModCategory::Font => text.overlay_slot_font,
        ModCategory::Announcer => text.overlay_slot_announcer,
        ModCategory::Ui => text.category_ui,
        ModCategory::Voiceover => text.category_voiceover,
        ModCategory::LoadingScreen => text.category_loading_screen,
        ModCategory::Vfx => text.category_vfx,
        ModCategory::Sfx => text.category_sfx,
        ModCategory::Other => text.category_other,
    }
}

#[must_use]
pub(crate) fn import_categories() -> Vec<ModCategory> {
    SINGLE_SLOTS
        .iter()
        .map(|(_, category)| *category)
        .chain(OTHER_CATEGORIES)
        .collect()
}

fn slot_entries<'a>(available: &'a ModCatalog, slot: &str) -> &'a [ModEntry] {
    match slot {
        "skin" => &available.skin,
        "map" => &available.map,
        "font" => &available.font,
        "announcer" => &available.announcer,
        _ => &available.others,
    }
}

fn slot_choice<'a>(selection: &'a ModSelectionView, slot: &str) -> Option<&'a str> {
    match slot {
        "skin" => selection.skin.as_deref(),
        "map" => selection.map.as_deref(),
        "font" => selection.font.as_deref(),
        "announcer" => selection.announcer.as_deref(),
        _ => None,
    }
}

fn line(kind: ModLineKind, text: &str) -> ModLine {
    ModLine {
        kind,
        text: text.into(),
        detail: SharedString::default(),
        slot: SharedString::default(),
        id: SharedString::default(),
        selected: false,
        radio: false,
        none: false,
    }
}

fn entry_line(
    entry: Option<&ModEntry>,
    slot: &str,
    selected: bool,
    radio: bool,
    text: &Text,
) -> ModLine {
    ModLine {
        kind: ModLineKind::Entry,
        text: entry.map_or(text.overlay_none, |e| e.name.as_str()).into(),
        detail: entry
            .and_then(|e| e.description.as_deref())
            .unwrap_or_default()
            .into(),
        slot: slot.into(),
        id: entry.map_or("", |e| e.id.as_str()).into(),
        selected,
        radio,
        none: entry.is_none(),
    }
}

#[must_use]
pub(crate) fn mod_lines(
    available: &ModCatalog,
    selection: &ModSelectionView,
    search: &str,
    text: &Text,
) -> Vec<ModLine> {
    let term = fold(search.trim());
    let matches = |entry: &&ModEntry| {
        term.is_empty()
            || fold(&entry.name).contains(&term)
            || entry
                .description
                .as_deref()
                .is_some_and(|d| fold(d).contains(&term))
    };
    let mut lines = Vec::new();
    if available.is_empty() {
        let mut empty = line(ModLineKind::Empty, text.overlay_no_mods_big);
        empty.detail = text.overlay_no_mods_sub.into();
        lines.push(empty);
    }

    let mut shown = 0;
    for (slot, category) in SINGLE_SLOTS {
        let entries: Vec<&ModEntry> = slot_entries(available, slot)
            .iter()
            .filter(matches)
            .collect();
        if entries.is_empty() {
            continue;
        }
        let chosen = slot_choice(selection, slot);
        lines.push(line(ModLineKind::Section, category_label(category, text)));
        lines.push(entry_line(None, slot, chosen.is_none(), true, text));
        for entry in &entries {
            lines.push(entry_line(
                Some(entry),
                slot,
                chosen == Some(entry.id.as_str()),
                true,
                text,
            ));
        }
        shown += entries.len();
    }

    let others: Vec<&ModEntry> = available.others.iter().filter(matches).collect();
    if !others.is_empty() {
        lines.push(line(ModLineKind::Section, text.overlay_slot_others));
        for category in OTHER_CATEGORIES {
            let group: Vec<&&ModEntry> = others.iter().filter(|e| e.category == category).collect();
            if group.is_empty() {
                continue;
            }
            lines.push(line(ModLineKind::Group, category_label(category, text)));
            for entry in group {
                let selected = selection.others.contains(&entry.id);
                lines.push(entry_line(Some(entry), "others", selected, false, text));
            }
        }
        shown += others.len();
    }

    if !available.is_empty() && shown == 0 && !term.is_empty() {
        lines.push(line(
            ModLineKind::Message,
            &fill(text.overlay_no_mods_match, "term", search.trim()),
        ));
    }
    lines
}

#[must_use]
pub(crate) fn toggle_mod(
    selection: &ModSelectionView,
    slot: &str,
    id: &str,
) -> Option<ModSelectionView> {
    let mut next = selection.clone();
    let choice = (!id.is_empty()).then(|| id.to_owned());
    match slot {
        "skin" => next.skin = choice,
        "map" => next.map = choice,
        "font" => next.font = choice,
        "announcer" => next.announcer = choice,
        "others" => {
            let id = choice?;
            if let Some(at) = next.others.iter().position(|other| *other == id) {
                next.others.remove(at);
            } else {
                next.others.push(id);
            }
        }
        _ => return None,
    }
    Some(next)
}

#[cfg(test)]
#[path = "overlay_model_tests.rs"]
mod tests;
