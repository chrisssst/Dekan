use serde::Deserialize;

use crate::client::LcuClient;
use crate::error::LcuError;

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ChampionChroma {
    #[serde(default)]
    pub id: u32,
    #[serde(default)]
    pub name: String,

    #[serde(default)]
    pub colors: Vec<String>,

    #[serde(default)]
    pub chroma_path: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct QuestTier {
    #[serde(default)]
    pub id: u32,
    #[serde(default)]
    pub name: String,

    #[serde(default)]
    pub tile_path: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct QuestSkinInfo {
    #[serde(default)]
    pub tiers: Vec<QuestTier>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ChampionSkin {
    #[serde(default)]
    pub id: u32,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub is_base: bool,
    #[serde(default)]
    pub chromas: Vec<ChampionChroma>,

    #[serde(default)]
    pub tile_path: Option<String>,

    #[serde(default)]
    pub quest_skin_info: Option<QuestSkinInfo>,
}

impl ChampionSkin {
    pub fn forms(&self) -> impl Iterator<Item = &QuestTier> + '_ {
        self.quest_skin_info
            .iter()
            .flat_map(|info| info.tiers.iter())
            .filter(move |tier| tier.id != self.id)
    }
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ChampionAssets {
    #[serde(default)]
    pub id: u32,
    #[serde(default)]
    pub name: String,

    #[serde(default)]
    pub alias: String,
    #[serde(default)]
    pub skins: Vec<ChampionSkin>,
}

impl ChampionAssets {
    #[must_use]
    pub fn name_of(&self, id: u32) -> Option<&str> {
        for skin in &self.skins {
            if skin.id == id {
                return Some(&skin.name);
            }
            if let Some(chroma) = skin.chromas.iter().find(|c| c.id == id) {
                return Some(&chroma.name);
            }
            if let Some(form) = skin.forms().find(|f| f.id == id) {
                return Some(&form.name);
            }
        }
        None
    }

    #[must_use]
    pub fn form(&self, id: u32) -> Option<&QuestTier> {
        self.skins
            .iter()
            .flat_map(ChampionSkin::forms)
            .find(|f| f.id == id)
    }

    #[must_use]
    pub fn base_skin_of(&self, id: u32) -> Option<u32> {
        self.skins
            .iter()
            .find(|skin| {
                skin.id == id
                    || skin.chromas.iter().any(|c| c.id == id)
                    || skin.forms().any(|f| f.id == id)
            })
            .map(|skin| skin.id)
    }

    #[must_use]
    pub fn chroma(&self, id: u32) -> Option<&ChampionChroma> {
        self.skins
            .iter()
            .flat_map(|s| s.chromas.iter())
            .find(|c| c.id == id)
    }
}

const MAX_ASSET_BYTES: usize = 1024 * 1024;

impl LcuClient {
    pub async fn get_champion_assets(&self, champion_id: u32) -> Result<ChampionAssets, LcuError> {
        let url = format!(
            "{}/lol-game-data/assets/v1/champions/{champion_id}.json",
            self.base_url()
        );
        let resp = self.http().get(&url).send().await?;

        if !resp.status().is_success() {
            return Err(LcuError::Parse(format!(
                "champion assets unavailable for {champion_id} (HTTP {})",
                resp.status().as_u16()
            )));
        }

        Ok(resp.json::<ChampionAssets>().await?)
    }

    pub async fn get_asset_bytes(&self, path: &str) -> Result<Vec<u8>, LcuError> {
        let url = format!("{}{path}", self.base_url());
        let resp = self.http().get(&url).send().await?;

        if !resp.status().is_success() {
            return Err(LcuError::Parse(format!(
                "asset unavailable at {path} (HTTP {})",
                resp.status().as_u16()
            )));
        }

        let bytes = resp.bytes().await?;
        if bytes.len() > MAX_ASSET_BYTES {
            return Err(LcuError::Parse(format!(
                "asset at {path} exceeds the {MAX_ASSET_BYTES}-byte cap ({} bytes)",
                bytes.len()
            )));
        }

        Ok(bytes.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ZED: &str = r##"{
        "id": 238,
        "name": "Zed",
        "skins": [
            { "id": 238000, "name": "Zed", "isBase": true,
              "tilePath": "/lol-game-data/assets/ASSETS/Characters/Zed/Skins/Base/ZedSquare.png" },
            { "id": 238001, "name": "Shockblade Zed", "isBase": false,
              "tilePath": "/lol-game-data/assets/ASSETS/Characters/Zed/Skins/Skin01/ZedSquare.png",
              "chromas": [
                { "id": 238004, "name": "Shockblade Zed (Rose Quartz)", "colors": ["#E58BA5", "#E58BA5"],
                  "chromaPath": "/lol-game-data/assets/v1/champion-chroma-images/238/238004.png" },
                { "id": 238005, "name": "Shockblade Zed (Catseye)", "colors": ["#FFEE59", "#FFEE59"] }
              ] },
            { "id": 238002, "name": "SKT T1 Zed", "isBase": false }
        ]
    }"##;

    fn zed() -> ChampionAssets {
        serde_json::from_str(ZED).expect("real response shape must parse")
    }

    #[test]
    fn test_parses_skins_and_chromas() {
        let assets = zed();
        assert_eq!(assets.name, "Zed");
        assert_eq!(assets.skins.len(), 3);
        assert!(assets.skins[0].is_base);
        assert_eq!(assets.skins[1].chromas.len(), 2);
        assert_eq!(assets.skins[2].chromas.len(), 0, "missing chromas is empty");
    }

    #[test]
    fn test_tile_path_parses_when_present_and_is_none_when_absent() {
        let assets = zed();
        assert_eq!(
            assets.skins[1].tile_path.as_deref(),
            Some("/lol-game-data/assets/ASSETS/Characters/Zed/Skins/Skin01/ZedSquare.png")
        );
        assert_eq!(
            assets.skins[2].tile_path, None,
            "a skin the client omitted the icon for must not become a parse error"
        );
    }

    #[test]
    fn test_chroma_path_parses_when_present_and_is_none_when_absent() {
        let assets = zed();
        assert_eq!(
            assets
                .chroma(238_004)
                .and_then(|c| c.chroma_path.as_deref()),
            Some("/lol-game-data/assets/v1/champion-chroma-images/238/238004.png")
        );
        assert_eq!(
            assets
                .chroma(238_005)
                .and_then(|c| c.chroma_path.as_deref()),
            None
        );
    }

    #[test]
    fn test_names_resolve_for_skins_and_chromas_alike() {
        let assets = zed();
        assert_eq!(assets.name_of(238001), Some("Shockblade Zed"));
        assert_eq!(
            assets.name_of(238005),
            Some("Shockblade Zed (Catseye)"),
            "a chroma id must resolve through the same door as a skin id"
        );
        assert_eq!(assets.name_of(999), None);
    }

    #[test]
    fn test_chroma_colors_are_available_for_the_swatch() {
        let assets = zed();
        let colors = &assets.chroma(238004).expect("known chroma").colors;
        assert_eq!(colors.first().map(String::as_str), Some("#E58BA5"));
    }

    #[test]
    fn test_unexpected_payload_does_not_fail_the_catalog() {
        let assets: ChampionAssets = serde_json::from_str("{}").expect("tolerates empty body");
        assert!(assets.skins.is_empty());
        assert_eq!(assets.name_of(1), None);
    }

    const SERAPHINE: &str = r##"{
        "id": 147, "name": "Seraphine", "alias": "Seraphine",
        "skins": [
            { "id": 147000, "name": "Seraphine", "isBase": true },
            { "id": 147001, "name": "K/DA ALL OUT Seraphine Indie",
              "questSkinInfo": { "name": "K/DA ALL OUT Seraphine", "tiers": [
                { "id": 147001, "name": "K/DA ALL OUT Seraphine Indie", "stage": 1 },
                { "id": 147002, "name": "K/DA ALL OUT Seraphine Rising Star", "stage": 2,
                  "tilePath": "/lol-game-data/assets/ASSETS/Characters/Seraphine/Skins/Skin02/Images/seraphine_splash_tile_2.jpg" },
                { "id": 147003, "name": "K/DA ALL OUT Seraphine Superstar", "stage": 3 }
              ] } }
        ]
    }"##;

    #[test]
    fn test_the_base_skin_of_a_skin_a_chroma_and_an_unknown_id() {
        let assets = zed();
        assert_eq!(
            assets.base_skin_of(238_001),
            Some(238_001),
            "a skin is its own base"
        );
        assert_eq!(
            assets.base_skin_of(238_005),
            Some(238_001),
            "a chroma belongs to its skin"
        );
        assert_eq!(assets.base_skin_of(238_002), Some(238_002));
        assert_eq!(assets.base_skin_of(238_999), None);
    }

    #[test]
    fn test_a_chroma_is_never_attributed_to_a_skin_with_forms() {
        let assets: ChampionAssets = serde_json::from_str(
            r##"{
                "id": 147, "name": "Seraphine", "alias": "Seraphine",
                "skins": [
                    { "id": 147001, "name": "Indie",
                      "questSkinInfo": { "tiers": [ { "id": 147001 }, { "id": 147002 } ] } },
                    { "id": 147010, "name": "Ocean Song",
                      "chromas": [ { "id": 147011, "name": "Ruby" } ] }
                ]
            }"##,
        )
        .expect("fixture parses");
        assert_eq!(assets.base_skin_of(147_011), Some(147_010));
        assert_eq!(assets.base_skin_of(147_002), Some(147_001));
    }

    #[test]
    fn test_quest_tiers_are_the_skin_forms_the_client_lists() {
        let assets: ChampionAssets = serde_json::from_str(SERAPHINE).expect("real shape parses");
        let forms: Vec<u32> = assets.skins[1].forms().map(|f| f.id).collect();
        assert_eq!(
            forms,
            vec![147_002, 147_003],
            "the tier equal to the skin is the skin itself"
        );
        assert_eq!(assets.base_skin_of(147_003), Some(147_001));
        assert_eq!(
            assets.name_of(147_002),
            Some("K/DA ALL OUT Seraphine Rising Star")
        );
        assert!(
            assets
                .form(147_002)
                .and_then(|f| f.tile_path.as_deref())
                .is_some()
        );
        assert!(assets.skins[0].forms().next().is_none());
    }
}
