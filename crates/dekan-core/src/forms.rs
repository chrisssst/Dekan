use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SpecialFormConfig {
    pub base_skin_id: u32,
    pub champion_id: u32,
    pub name: &'static str,
    pub button_folder: &'static str,
    pub form_ids: &'static [u32],
    pub form_names: &'static [&'static str],
}

/// Static table of all known special skin forms in League of Legends.
pub static SPECIAL_FORMS: &[SpecialFormConfig] = &[
    // Gun Goddess Miss Fortune
    SpecialFormConfig {
        base_skin_id: 21016,
        champion_id: 21,
        name: "Gun Goddess Miss Fortune",
        button_folder: "ggmf_buttons",
        form_ids: &[21016, 21997, 21998, 21999],
        form_names: &["Scarlet Fair", "Zero Hour", "Royal Arms", "Starswarm"],
    },
    // DJ Sona
    SpecialFormConfig {
        base_skin_id: 37006,
        champion_id: 37,
        name: "DJ Sona",
        button_folder: "djsona_buttons",
        form_ids: &[37006, 37998, 37999],
        form_names: &["Kinetic", "Concussive", "Ethereal"],
    },
    // Spirit Blossom Morgana
    SpecialFormConfig {
        base_skin_id: 25080,
        champion_id: 25,
        name: "Spirit Blossom Morgana",
        button_folder: "sbmorg_buttons",
        form_ids: &[25080, 25999],
        form_names: &["Default", "Transformed"],
    },
    // Sahn-Uzal Mordekaiser
    SpecialFormConfig {
        base_skin_id: 82054,
        champion_id: 82,
        name: "Sahn-Uzal Mordekaiser",
        button_folder: "uzal_buttons",
        form_ids: &[82054, 82998, 82999],
        form_names: &["Default", "Form 1", "Form 2"],
    },
    // Radiant Sett
    SpecialFormConfig {
        base_skin_id: 875066,
        champion_id: 875,
        name: "Radiant Sett",
        button_folder: "radiantsett_buttons",
        form_ids: &[875066, 875998, 875999],
        form_names: &["Default", "Form 2", "Form 3"],
    },
    // K/DA ALL OUT Seraphine
    SpecialFormConfig {
        base_skin_id: 147001,
        champion_id: 147,
        name: "K/DA ALL OUT Seraphine",
        button_folder: "kdasera_buttons",
        form_ids: &[147001, 147002, 147003],
        form_names: &["Indie", "Rising Star", "Superstar"],
    },
    // Arcane Fractured Jinx
    SpecialFormConfig {
        base_skin_id: 222060,
        champion_id: 222,
        name: "Arcane Fractured Jinx",
        button_folder: "arcanejinx_buttons",
        form_ids: &[222060, 222998, 222999],
        form_names: &["Default", "Form 1", "Form 2"],
    },
    // Uzi Kaisa
    SpecialFormConfig {
        base_skin_id: 145070,
        champion_id: 145,
        name: "Uzi Kaisa",
        button_folder: "uzikaisa_buttons",
        form_ids: &[145070, 145071, 145999],
        form_names: &["Default", "Form 1", "Form 2"],
    },
    // Soul Fighter Viego
    SpecialFormConfig {
        base_skin_id: 234043,
        champion_id: 234,
        name: "Soul Fighter Viego",
        button_folder: "rrviego_buttons",
        form_ids: &[234043, 234994, 234995, 234996, 234997, 234998, 234999],
        form_names: &[
            "Default", "Form 2", "Form 3", "Form 4", "Form 5", "Form 6", "Form 7",
        ],
    },
    // Immortalized Legend Ahri (Hall of Legends)
    SpecialFormConfig {
        base_skin_id: 103085,
        champion_id: 103,
        name: "Immortalized Legend Ahri",
        button_folder: "fakerahri_buttons",
        form_ids: &[103085, 103086, 103087],
        form_names: &["Risen Legend", "Immortalized Legend", "Signature Edition"],
    },
];

/// Display name of a form entry.
#[must_use]
pub fn form_display_name(id: u32) -> Option<String> {
    let config = find_form_config(id)?;
    if config.base_skin_id == id {
        return None;
    }
    let index = config.form_ids.iter().position(|form| *form == id)?;
    let form = config.form_names.get(index)?;
    Some(format!("{} — {form}", config.name))
}

#[must_use]
pub fn find_form_config(skin_id: u32) -> Option<&'static SpecialFormConfig> {
    SPECIAL_FORMS
        .iter()
        .find(|cfg| cfg.base_skin_id == skin_id || cfg.form_ids.contains(&skin_id))
}

#[must_use]
pub fn is_special_form_skin(skin_id: u32) -> bool {
    find_form_config(skin_id).is_some()
}

#[must_use]
pub fn resolve_base_skin_for_form(form_skin_id: u32) -> u32 {
    find_form_config(form_skin_id)
        .map(|cfg| cfg.base_skin_id)
        .unwrap_or(form_skin_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_form_has_a_name() {
        for config in SPECIAL_FORMS {
            assert_eq!(
                config.form_ids.len(),
                config.form_names.len(),
                "{}: ids and names must pair up",
                config.name
            );
            assert!(
                config.form_ids.contains(&config.base_skin_id),
                "{}",
                config.name
            );
        }
    }

    #[test]
    fn a_form_is_named_after_its_skin_and_the_base_is_not() {
        assert_eq!(
            form_display_name(21997).as_deref(),
            Some("Gun Goddess Miss Fortune — Zero Hour")
        );
        assert_eq!(form_display_name(21016), None);
        assert_eq!(form_display_name(1), None);
    }

    #[test]
    fn test_ggmf_forms_lookup() {
        let cfg = find_form_config(21016).expect("GGMF found by base id");
        assert_eq!(cfg.champion_id, 21);
        assert_eq!(cfg.form_ids.len(), 4);

        let cfg_sub = find_form_config(21999).expect("GGMF found by sub form id");
        assert_eq!(cfg_sub.base_skin_id, 21016);
        assert_eq!(resolve_base_skin_for_form(21999), 21016);
    }

    #[test]
    fn test_dj_sona_forms() {
        assert!(is_special_form_skin(37006));
        assert!(is_special_form_skin(37998));
        assert!(!is_special_form_skin(37001));
    }

    #[test]
    fn test_faker_ahri_forms() {
        assert_eq!(resolve_base_skin_for_form(103087), 103085);
    }
}
