use super::*;

#[test]
fn client_locales_map_by_language_family() {
    assert_eq!(Language::from_locale("tr-TR"), Some(Language::Turkish));
    assert_eq!(Language::from_locale("pt_BR"), Some(Language::Portuguese));
    assert_eq!(Language::from_locale("es_MX"), Some(Language::Spanish));
    assert_eq!(Language::from_locale("en-GB"), Some(Language::English));
    assert_eq!(Language::from_locale("ko_KR"), None);
    assert_eq!(Language::from_locale(""), None);
}

#[cfg(windows)]
#[test]
fn windows_primary_language_ids_map_to_dictionaries() {
    assert_eq!(Language::from_primary_lang_id(0x16), Language::Portuguese);
    assert_eq!(Language::from_primary_lang_id(0x0a), Language::Spanish);
    assert_eq!(Language::from_primary_lang_id(0x09), Language::English);
    assert_eq!(Language::from_primary_lang_id(0x11), Language::English);
}

#[test]
fn templates_are_filled_and_every_dictionary_keeps_its_placeholders() {
    for text in [&PORTUGUESE, &SPANISH, &ENGLISH] {
        assert!(fill(text.party_in_room, "n", "3").contains('3'));
        for (template, key) in [
            (text.party_in_room, "n"),
            (text.star_gate_missing_star, "user"),
            (text.star_gate_unreachable, "error"),
            (text.party_copy_failed_body, "code"),
            (text.party_unavailable_body, "reason"),
            (text.import_refused, "reason"),
            (text.import_not_a_mod, "error"),
            (text.import_io_error, "error"),
            (text.party_invalid_code, "error"),
            (text.party_join_clipboard_error, "error"),
            (text.update_available_title, "version"),
            (text.update_offer_title, "version"),
            (text.update_offer_body, "version"),
            (text.panel_update_line, "version"),
            (text.panel_update_line, "current"),
            (text.overlay_no_results_sub, "term"),
            (text.overlay_no_library_big, "champion"),
            (text.overlay_skin_many, "count"),
            (text.overlay_no_mods_match, "term"),
            (text.detail_custom_mods, "repaired"),
            (text.detail_custom_mods, "refused"),
            (text.custom_mods_repaired_body, "n"),
            (text.custom_mods_refused_body, "n"),
            (text.custom_mods_restored_body, "n"),
            (text.custom_mods_restore_failed, "error"),
        ] {
            assert!(
                template.contains(&format!("{{{key}}}")),
                "{template:?} must keep {{{key}}}"
            );
        }
    }
}

fn literals(source: &str) -> Vec<String> {
    source
        .replace("\\\"", "'")
        .split('"')
        .skip(1)
        .step_by(2)
        .map(str::to_owned)
        .collect()
}

#[test]
fn no_text_carries_emoji_symbols_or_shouting_capitals() {
    for source in [
        include_str!("tr.rs"),
        include_str!("pt.rs"),
        include_str!("en.rs"),
        include_str!("es.rs"),
    ] {
        for text in &literals(source) {
            assert!(
                !text
                    .chars()
                    .any(|c| matches!(u32::from(c), 0x2190..=0x2BFF | 0xFE0F | 0x1F000..=0x1FFFF)),
                "{text:?} draws a symbol; icons are vector paths in the interface"
            );
            let letters: Vec<char> = text.chars().filter(|c| c.is_alphabetic()).collect();
            assert!(
                !(letters.len() > 4 && letters.iter().all(|c| c.is_uppercase())),
                "{text:?} is written in capitals; the interface uses sentence case"
            );
        }
    }
}
