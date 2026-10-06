use serde::{Deserialize, Serialize};

use crate::mods::ModSelectionView;
use crate::selection::{ChampionId, ChromaId, SkinId};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum OverlayCommand {
    Select { id: u32 },

    Clear,

    Random,

    SetMods { selection: ModSelectionView },

    OpenModsFolder,

    ImportMod { category: crate::mods::ModCategory },

    ChromaPreview { id: u32 },
}

impl OverlayCommand {
    pub fn parse(payload: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(payload)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OverlayTarget {
    pub champion_id: ChampionId,

    pub skin_id: SkinId,

    pub chroma_id: Option<ChromaId>,
}

impl OverlayTarget {
    #[must_use]
    pub fn package_entry_id(&self) -> u32 {
        self.chroma_id.unwrap_or(self.skin_id)
    }

    #[must_use]
    pub fn matches_champion(&self, champion_id: ChampionId) -> bool {
        self.champion_id == champion_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_message_the_ui_actually_sends() {
        assert_eq!(
            OverlayCommand::parse(r#"{"type":"select","id":238001}"#)
                .expect("the UI's own message must parse"),
            OverlayCommand::Select { id: 238_001 }
        );
        assert_eq!(
            OverlayCommand::parse(r#"{"type":"clear"}"#).expect("clear must parse"),
            OverlayCommand::Clear
        );

        assert_eq!(
            OverlayCommand::parse(
                r#"{"type":"setMods","selection":{"map":"dekan:maps/Winter","others":["dekan:ui/HUD"]}}"#
            )
            .expect("setMods must parse"),
            OverlayCommand::SetMods {
                selection: ModSelectionView {
                    map: Some("dekan:maps/Winter".into()),
                    others: vec!["dekan:ui/HUD".into()],
                    ..Default::default()
                }
            }
        );
        assert_eq!(
            OverlayCommand::parse(r#"{"type":"openModsFolder"}"#).expect("must parse"),
            OverlayCommand::OpenModsFolder
        );
        assert_eq!(
            OverlayCommand::parse(r#"{"type":"random"}"#).expect("must parse"),
            OverlayCommand::Random
        );
        assert_eq!(
            OverlayCommand::parse(r#"{"type":"importMod","category":"loadingScreen"}"#)
                .expect("must parse"),
            OverlayCommand::ImportMod {
                category: crate::mods::ModCategory::LoadingScreen
            }
        );
        assert_eq!(
            OverlayCommand::parse(r#"{"type":"chromaPreview","id":238004}"#).expect("must parse"),
            OverlayCommand::ChromaPreview { id: 238_004 }
        );
        assert!(
            OverlayCommand::parse(r#"{"type":"chromaPreview","path":"/x"}"#).is_err(),
            "a preview request carries an id, never a path"
        );
        assert!(
            OverlayCommand::parse(r#"{"type":"importMod","category":"C:/Windows"}"#).is_err(),
            "a category outside the known ten is refused"
        );
    }

    #[test]
    fn rejects_messages_it_does_not_understand() {
        assert!(
            OverlayCommand::parse(r#"{"type":"inject","id":1}"#).is_err(),
            "an unknown command must not be silently accepted"
        );
        assert!(
            OverlayCommand::parse(r#"{"type":"select"}"#).is_err(),
            "a select without an id has no target and must fail"
        );
        assert!(
            OverlayCommand::parse("not json at all").is_err(),
            "garbage must fail rather than resolve to a default"
        );
    }

    #[test]
    fn chroma_is_the_entry_that_gets_installed() {
        let skin = OverlayTarget {
            champion_id: 238,
            skin_id: 238_001,
            chroma_id: None,
        };
        assert_eq!(skin.package_entry_id(), 238_001);

        let chroma = OverlayTarget {
            champion_id: 238,
            skin_id: 238_001,
            chroma_id: Some(238_015),
        };
        assert_eq!(
            chroma.package_entry_id(),
            238_015,
            "a chroma has its own package; injecting the parent skin would show the wrong colours"
        );
    }

    #[test]
    fn a_target_never_crosses_champions() {
        let target = OverlayTarget {
            champion_id: 238,
            skin_id: 238_001,
            chroma_id: None,
        };
        assert!(target.matches_champion(238));
        assert!(
            !target.matches_champion(103),
            "a Zed pick must not be injected for Ahri after a bench swap"
        );
    }
}
