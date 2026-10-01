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
        }
        None
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
}
