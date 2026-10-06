use std::path::{Path, PathBuf};

use dekan_core::library::{ChampionLibrary, scan_champion};
use dekan_core::overlay::OverlayTarget;
use dekan_lcu::champion_assets::ChampionAssets;
use serde::Serialize;
use tracing::{debug, info, warn};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CatalogChroma {
    pub id: u32,
    pub name: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,

    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub form: bool,

    #[serde(skip)]
    pub preview_path: Option<String>,

    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub has_preview: bool,
}

impl CatalogChroma {
    fn with_preview(mut self, path: Option<&str>) -> Self {
        self.preview_path = path.filter(|p| p.starts_with('/')).map(str::to_owned);
        self.has_preview = self.preview_path.is_some();
        self
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CatalogSkin {
    pub id: u32,
    pub name: String,

    pub name_unknown: bool,
    pub chromas: Vec<CatalogChroma>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub tile: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Catalog {
    pub champion_id: u32,
    pub champion_name: String,

    #[serde(skip)]
    pub alias: Option<String>,
    pub skins: Vec<CatalogSkin>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub quote: Option<String>,

    pub mods: ModsPanel,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub notice: Option<CatalogNotice>,

    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub classic: bool,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CatalogNotice {
    ToolsMissing,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModsPanel {
    pub available: dekan_core::mods::ModCatalog,
    pub selection: dekan_core::mods::ModSelectionView,
}

impl Catalog {
    #[must_use]
    pub fn entry_count(&self) -> usize {
        self.skins.iter().map(|s| 1 + s.chromas.len()).sum()
    }

    pub fn roll_random(&self, mut pick: impl FnMut(usize) -> usize) -> Option<OverlayTarget> {
        let candidates: Vec<&CatalogSkin> = self
            .skins
            .iter()
            .filter(|skin| {
                !dekan_core::selection::SelectionMode::is_base_skin(skin.id, self.champion_id)
            })
            .collect();
        if candidates.is_empty() {
            return None;
        }
        let skin = candidates[pick(candidates.len()) % candidates.len()];
        let options = 1 + skin.chromas.len();
        let entry = match pick(options) % options {
            0 => skin.id,
            n => skin.chromas[n - 1].id,
        };
        self.resolve_target(entry)
    }

    #[must_use]
    pub fn resolve_target(&self, entry_id: u32) -> Option<OverlayTarget> {
        for skin in &self.skins {
            if skin.id == entry_id {
                return Some(OverlayTarget {
                    champion_id: self.champion_id,
                    skin_id: skin.id,
                    chroma_id: None,
                });
            }
            if let Some(chroma) = skin.chromas.iter().find(|c| c.id == entry_id) {
                return Some(OverlayTarget {
                    champion_id: self.champion_id,
                    skin_id: skin.id,
                    chroma_id: Some(chroma.id),
                });
            }
        }
        None
    }

    #[must_use]
    pub fn chroma_preview_paths(&self) -> Vec<(u32, String)> {
        self.skins
            .iter()
            .flat_map(|s| s.chromas.iter())
            .filter_map(|c| c.preview_path.clone().map(|path| (c.id, path)))
            .collect()
    }

    #[must_use]
    pub fn chroma_preview_path(&self, chroma_id: u32) -> Option<&str> {
        self.skins
            .iter()
            .flat_map(|s| s.chromas.iter())
            .find(|c| c.id == chroma_id)
            .and_then(|c| c.preview_path.as_deref())
    }
}

fn client_form(form: &dekan_lcu::champion_assets::QuestTier) -> CatalogChroma {
    CatalogChroma {
        id: form.id,
        name: if form.name.is_empty() {
            fallback_name(form.id)
        } else {
            form.name.clone()
        },
        color: None,
        form: true,
        preview_path: None,
        has_preview: false,
    }
    .with_preview(form.tile_path.as_deref())
}

fn fallback_name(id: u32) -> String {
    format!("Skin {id}")
}

#[must_use]
pub fn build_catalog(library: &ChampionLibrary, assets: Option<&ChampionAssets>) -> Catalog {
    let champion_name = assets
        .map(|a| a.name.clone())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| format!("#{}", library.champion_id));

    let alias = assets.map(|a| a.alias.clone()).filter(|a| !a.is_empty());

    let skins: Vec<CatalogSkin> = if !library.skins.is_empty() {
        library
            .skins
            .iter()
            .filter(|skin| {
                !dekan_core::selection::SelectionMode::is_base_skin(skin.id, library.champion_id)
            })
            .map(|skin| {
                let named = assets.and_then(|a| a.name_of(skin.id));
                let chromas = skin
                    .chromas
                    .iter()
                    .map(|chroma| {
                        let chroma_meta = assets.and_then(|a| a.chroma(chroma.id));
                        let form_meta = assets.and_then(|a| a.form(chroma.id));
                        let client_name = assets
                            .and_then(|a| a.name_of(chroma.id))
                            .filter(|n| !n.is_empty())
                            .map(str::to_owned);
                        CatalogChroma {
                            id: chroma.id,
                            form: form_meta.is_some(),
                            name: client_name.unwrap_or_else(|| fallback_name(chroma.id)),
                            color: chroma_meta.and_then(|c| c.colors.first().cloned()),
                            preview_path: None,
                            has_preview: false,
                        }
                        .with_preview(
                            chroma_meta
                                .and_then(|c| c.chroma_path.as_deref())
                                .or_else(|| form_meta.and_then(|f| f.tile_path.as_deref())),
                        )
                    })
                    .chain(
                        assets
                            .and_then(|a| a.skins.iter().find(|s| s.id == skin.id))
                            .into_iter()
                            .flat_map(|client_skin| client_skin.forms())
                            .filter(|form| skin.chromas.iter().all(|c| c.id != form.id))
                            .map(client_form),
                    )
                    .collect();

                CatalogSkin {
                    id: skin.id,
                    name: named
                        .filter(|n| !n.is_empty())
                        .map(str::to_owned)
                        .unwrap_or_else(|| fallback_name(skin.id)),
                    name_unknown: named.is_none_or(str::is_empty),
                    chromas,
                    tile: None,
                }
            })
            .collect()
    } else if let Some(assets) = assets {
        assets
            .skins
            .iter()
            .filter(|skin| {
                !skin.is_base
                    && !dekan_core::selection::SelectionMode::is_base_skin(
                        skin.id,
                        library.champion_id,
                    )
            })
            .map(|skin| {
                let chromas = skin
                    .chromas
                    .iter()
                    .map(|chroma| {
                        CatalogChroma {
                            id: chroma.id,
                            form: false,
                            name: if chroma.name.is_empty() {
                                fallback_name(chroma.id)
                            } else {
                                chroma.name.clone()
                            },
                            color: chroma.colors.first().cloned(),
                            preview_path: None,
                            has_preview: false,
                        }
                        .with_preview(chroma.chroma_path.as_deref())
                    })
                    .chain(skin.forms().map(client_form))
                    .collect();

                CatalogSkin {
                    id: skin.id,
                    name: if !skin.name.is_empty() {
                        skin.name.clone()
                    } else {
                        fallback_name(skin.id)
                    },
                    name_unknown: skin.name.is_empty(),
                    chromas,
                    tile: None,
                }
            })
            .collect()
    } else {
        Vec::new()
    };

    Catalog {
        champion_id: library.champion_id,
        champion_name,
        alias,
        skins,
        locale: None,
        quote: champion_quote(library.champion_id).map(str::to_owned),
        mods: ModsPanel::default(),
        notice: None,
        classic: false,
    }
}

fn champion_quote(champion_id: u32) -> Option<&'static str> {
    match champion_id {
        21 => Some("Eu sempre atiro primeiro!"),
        _ => None,
    }
}

pub async fn load_catalog(library_root: PathBuf, champion_id: u32) -> Catalog {
    let library = match tokio::task::spawn_blocking(move || {
        scan_champion(&library_root, champion_id)
    })
    .await
    {
        Ok(library) => library,
        Err(e) => {
            warn!(error = %e, champion_id, "Library scan task failed; catalog will be empty");
            ChampionLibrary {
                champion_id,
                skins: Vec::new(),
            }
        }
    };

    let fetched = fetch_assets(champion_id).await;
    if fetched.is_none() {
        debug!(champion_id, "No client metadata; catalog falls back to ids");
    }

    let assets = fetched.as_ref().map(|(assets, _)| assets);
    let mut catalog = build_catalog(&library, assets);

    let mut tiles_fetched = 0usize;
    if let Some((assets, client)) = &fetched {
        tiles_fetched = attach_tiles(&mut catalog, assets, client).await;
        catalog.locale = resolve_locale(client).await;
    }

    if catalog.alias.is_none() || catalog.skins.is_empty() {
        if let Some(game_dir) = dekan_platform::paths::discover_game_dir() {
            if catalog.alias.is_none() {
                let installed = game_dir.clone();
                catalog.alias = tokio::task::spawn_blocking(move || {
                    dekan_classic::client_data::champion_alias(&installed, champion_id)
                })
                .await
                .ok()
                .flatten();
            }
            if let Some(alias) = catalog.alias.clone().filter(|_| catalog.skins.is_empty()) {
                if let Ok(champ) =
                    dekan_classic::generator::StandardChampion::open(&game_dir, &alias)
                {
                    let numbers = champ.skin_numbers(1000);
                    for n in numbers {
                        let skin_id = champion_id * 1000 + n;
                        catalog.skins.push(CatalogSkin {
                            id: skin_id,
                            name: format!("{alias} #{n}"),
                            name_unknown: true,
                            chromas: Vec::new(),
                            tile: None,
                        });
                    }
                }
            }
        }
    }

    debug!(
        champion_id,
        skins = catalog.skins.len(),
        entries = catalog.entry_count(),
        tiles_fetched,
        locale = catalog.locale.as_deref().unwrap_or("-"),
        "Catalog assembled"
    );
    catalog
}

#[must_use]
pub fn build_classic_catalog(
    classic_champion_id: u32,
    assets: Option<&ChampionAssets>,
    numbers: &std::collections::BTreeSet<u32>,
) -> (Catalog, usize) {
    let regular =
        dekan_classic::builder::ClassicIdMapper::normalize_champion_id(classic_champion_id);
    let offerable = |id: u32| {
        let number = id % 1000;
        number != 0 && numbers.contains(&number)
    };

    let mut dropped = 0usize;
    let skins: Vec<CatalogSkin> = match assets {
        Some(assets) => assets
            .skins
            .iter()
            .filter(|skin| !skin.is_base)
            .filter_map(|skin| {
                let chromas: Vec<CatalogChroma> = skin
                    .chromas
                    .iter()
                    .filter(|c| offerable(c.id))
                    .map(|c| {
                        CatalogChroma {
                            id: c.id,
                            name: if c.name.is_empty() {
                                fallback_name(c.id)
                            } else {
                                c.name.clone()
                            },
                            color: c.colors.first().cloned(),
                            form: false,
                            preview_path: None,
                            has_preview: false,
                        }
                        .with_preview(c.chroma_path.as_deref())
                    })
                    .chain(skin.forms().filter(|f| offerable(f.id)).map(client_form))
                    .collect();
                dropped += (skin.chromas.len() + skin.forms().count()) - chromas.len();
                if !offerable(skin.id) {
                    dropped += 1 + chromas.len();
                    return None;
                }
                Some(CatalogSkin {
                    id: skin.id,
                    name: if skin.name.is_empty() {
                        fallback_name(skin.id)
                    } else {
                        skin.name.clone()
                    },
                    name_unknown: skin.name.is_empty(),
                    chromas,
                    tile: None,
                })
            })
            .collect(),

        None => numbers
            .iter()
            .copied()
            .filter(|n| *n != 0)
            .map(|n| {
                let id = regular * 1000 + n;
                CatalogSkin {
                    id,
                    name: fallback_name(id),
                    name_unknown: true,
                    chromas: Vec::new(),
                    tile: None,
                }
            })
            .collect(),
    };

    let champion_name = assets
        .map(|assets| assets.name.clone())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| format!("#{regular}"));

    let catalog = Catalog {
        champion_id: classic_champion_id,

        champion_name,
        classic: true,

        alias: None,
        skins,
        locale: None,
        quote: None,
        mods: ModsPanel::default(),
        notice: None,
    };
    (catalog, dropped)
}

pub async fn load_classic_catalog(
    game_dir: PathBuf,
    library_root: PathBuf,
    classic_champion_id: u32,
) -> Catalog {
    use dekan_classic::builder::ClassicIdMapper;
    use dekan_classic::generator::{ClassicChampion, resolve_alias_with_id};

    let regular = ClassicIdMapper::normalize_champion_id(classic_champion_id);
    let fetched = match fetch_assets(classic_champion_id).await {
        Some(classic) if classic.0.skins.iter().any(|skin| !skin.is_base) => Some(classic),
        _ => {
            debug!(
                champion_id = classic_champion_id,
                regular, "No Classic skin list from the client; using the regular champion's"
            );
            fetch_assets(regular).await
        }
    };

    let client_classic_alias = fetched
        .as_ref()
        .map(|(assets, _)| assets.alias.clone())
        .filter(|alias| alias.to_ascii_lowercase().starts_with("jade_"));
    let library_dir = library_root.join(regular.to_string());
    let scan = tokio::task::spawn_blocking(move || {
        let alias = resolve_alias_with_id(&game_dir, None, Some(regular), &library_dir)?;
        let classic_alias = client_classic_alias.or_else(|| {
            dekan_classic::client_data::champion_alias(&game_dir, classic_champion_id)
        });
        let champion = match ClassicChampion::open(&game_dir, &alias) {
            Ok(champion) => champion.with_client_character(classic_alias.as_deref()),
            Err(e) => {
                warn!(alias = %alias, error = %e, "Champion WAD could not be opened for Rift Classic");
                return None;
            }
        };
        let numbers: std::collections::BTreeSet<u32> = champion
            .skin_numbers(champion.main_character(), 1000)
            .into_iter()
            .collect();
        Some((alias, numbers))
    })
    .await;

    let (alias, numbers) = match scan {
        Ok(Some(found)) => found,
        Ok(None) => {
            warn!(
                champion_id = classic_champion_id,
                regular, "No champion WAD alias found for Rift Classic; nothing can be offered"
            );
            (String::new(), std::collections::BTreeSet::new())
        }
        Err(e) => {
            warn!(error = %e, "Rift Classic scan task failed; nothing can be offered");
            (String::new(), std::collections::BTreeSet::new())
        }
    };
    let (mut catalog, dropped) = build_classic_catalog(
        classic_champion_id,
        fetched.as_ref().map(|(assets, _)| assets),
        &numbers,
    );
    if let Some((assets, client)) = &fetched {
        attach_tiles(&mut catalog, assets, client).await;
        catalog.locale = resolve_locale(client).await;
    }

    info!(
        champion_id = classic_champion_id,
        regular,
        alias = %alias,
        jade_numbers = numbers.len(),
        offered = catalog.entry_count(),
        not_in_classic = dropped,
        "Rift Classic catalog built from the installed game"
    );
    catalog
}

async fn lcu_client() -> Option<dekan_lcu::client::LcuClient> {
    let lockfile = tokio::task::spawn_blocking(|| dekan_lcu::lockfile::Lockfile::discover(None))
        .await
        .ok()?
        .ok()?;

    dekan_lcu::client::LcuClient::new(&lockfile, dekan_lcu::client::DEFAULT_LCU_TIMEOUT).ok()
}

const PREVIEW_FETCHES: usize = 4;

pub type PreviewFetches = futures_util::stream::BoxStream<'static, (u32, Option<String>)>;

#[must_use]
pub fn chroma_preview_fetches(previews: Vec<(u32, String)>) -> PreviewFetches {
    use futures_util::StreamExt;
    futures_util::stream::once(lcu_client())
        .flat_map(move |client| {
            futures_util::stream::iter(previews.clone())
                .map(move |(id, path)| {
                    let client = client.clone();
                    async move {
                        let uri = match client {
                            Some(client) => fetch_chroma_preview(&client, &path).await,
                            None => None,
                        };
                        (id, uri)
                    }
                })
                .buffer_unordered(PREVIEW_FETCHES)
        })
        .boxed()
}

async fn fetch_chroma_preview(client: &dekan_lcu::client::LcuClient, path: &str) -> Option<String> {
    match client.get_asset_bytes(path).await {
        Ok(bytes) if !bytes.is_empty() => Some(tile_data_uri(path, &bytes)),
        Ok(_) => None,
        Err(e) => {
            debug!(path, error = %e, "Chroma preview unavailable");
            None
        }
    }
}

async fn fetch_assets(champion_id: u32) -> Option<(ChampionAssets, dekan_lcu::client::LcuClient)> {
    let client = lcu_client().await?;

    match client.get_champion_assets(champion_id).await {
        Ok(assets) => Some((assets, client)),
        Err(e) => {
            debug!(error = %e, champion_id, "Champion assets unavailable");
            None
        }
    }
}

static LOCALE_CACHE: std::sync::RwLock<Option<String>> = std::sync::RwLock::new(None);

pub fn invalidate_locale_cache() {
    if let Ok(mut lock) = LOCALE_CACHE.write() {
        *lock = None;
    }
    dekan_platform::i18n::reset_active_language();
}

async fn resolve_locale(client: &dekan_lcu::client::LcuClient) -> Option<String> {
    if let Ok(lock) = LOCALE_CACHE.read() {
        if let Some(locale) = lock.as_ref() {
            return Some(locale.clone());
        }
    }

    match client.get_region_locale().await {
        Ok(locale) if !locale.is_empty() => {
            if let Ok(mut lock) = LOCALE_CACHE.write() {
                *lock = Some(locale.clone());
            }
            dekan_platform::i18n::set_active_locale(&locale);
            info!(locale = %locale, "Client locale detected; overlay and UI follow it");
            Some(locale)
        }
        Ok(_) => None,
        Err(e) => {
            debug!(error = %e, "Client locale unavailable; overlay keeps its default language");
            None
        }
    }
}

async fn attach_tiles(
    catalog: &mut Catalog,
    assets: &ChampionAssets,
    client: &dekan_lcu::client::LcuClient,
) -> usize {
    let mut fetched = 0usize;
    for skin in &mut catalog.skins {
        let Some(path) = assets
            .skins
            .iter()
            .find(|s| s.id == skin.id)
            .and_then(|s| s.tile_path.as_deref())
        else {
            continue;
        };

        match client.get_asset_bytes(path).await {
            Ok(bytes) if !bytes.is_empty() => {
                skin.tile = Some(tile_data_uri(path, &bytes));
                fetched += 1;
            }
            Ok(_) => {}
            Err(e) => {
                debug!(skin_id = skin.id, path, error = %e, "Skin tile unavailable");
            }
        }
    }
    fetched
}

fn tile_data_uri(path: &str, bytes: &[u8]) -> String {
    let mime = if path.ends_with(".jpg") || path.ends_with(".jpeg") {
        "image/jpeg"
    } else {
        "image/png"
    };
    use base64::Engine;
    format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

pub fn catalog_json(catalog: &Catalog) -> Result<String, serde_json::Error> {
    serde_json::to_string(catalog)
}

#[must_use]
pub fn resolve_library_root(configured: &Path) -> PathBuf {
    configured.to_path_buf()
}

#[cfg(test)]
#[path = "catalog_tests.rs"]
mod tests;
