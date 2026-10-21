use std::path::{Path, PathBuf};
use std::sync::Arc;

use dekan_core::library::{ChampionLibrary, scan_champion};
pub use dekan_core::overlay::{Catalog, CatalogChroma, CatalogNotice, CatalogSkin, ModsPanel};
use dekan_lcu::champion_assets::ChampionAssets;
use tracing::{debug, info, warn};

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
            .filter(|skin| !dekan_core::selection::is_base_skin(skin.id, library.champion_id))
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
                !skin.is_base && !dekan_core::selection::is_base_skin(skin.id, library.champion_id)
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
        lobby: Vec::new(),
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
                catalog.alias = match tokio::task::spawn_blocking(move || {
                    dekan_classic::client_data::champion_alias(&installed, champion_id)
                })
                .await
                {
                    Ok(alias) => alias,
                    Err(e) => {
                        warn!(champion_id, error = %e, "The champion alias lookup task failed");
                        None
                    }
                };
            }
            if let Some(alias) = catalog.alias.clone().filter(|_| catalog.skins.is_empty()) {
                match dekan_classic::generator::StandardChampion::open(&game_dir, &alias) {
                    Ok(champ) => {
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
                    Err(e) => warn!(
                        champion_id,
                        alias = %alias,
                        error = %e,
                        "Champion WAD could not be opened; no skins can be listed from the installed game"
                    ),
                }
            }
        } else {
            debug!(
                champion_id,
                "The game folder is not known; skins are not listed from the installed game"
            );
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

async fn lcu_client() -> Option<dekan_lcu::client::LcuClient> {
    dekan_lcu::client::LcuClient::discover()
        .await
        .inspect_err(|e| debug!(error = %e, "The League client could not be reached"))
        .ok()
}

pub async fn lobby_champions(ids: &[u32]) -> Vec<dekan_core::overlay::LobbyChampion> {
    let client = lcu_client().await;
    let client = client.as_ref();
    futures_util::future::join_all(ids.iter().map(|&id| async move {
        let name = match client {
            Some(client) => match client.get_champion_assets(id).await {
                Ok(assets) if !assets.name.is_empty() => assets.name,
                Ok(_) => format!("#{id}"),
                Err(e) => {
                    warn!(champion_id = id, error = %e, "Lobby champion name unavailable");
                    format!("#{id}")
                }
            },
            None => format!("#{id}"),
        };
        dekan_core::overlay::LobbyChampion { id, name }
    }))
    .await
}

async fn fetch_assets(champion_id: u32) -> Option<(ChampionAssets, dekan_lcu::client::LcuClient)> {
    let client = match dekan_lcu::client::LcuClient::discover().await {
        Ok(client) => client,
        Err(e) => {
            warn!(error = %e, champion_id, "The League client could not be reached; names and tiles fall back to ids");
            return None;
        }
    };

    match client.get_champion_assets(champion_id).await {
        Ok(assets) => Some((assets, client)),
        Err(e) => {
            warn!(error = %e, champion_id, "Champion assets unavailable; names and tiles fall back to ids");
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
            warn!(error = %e, "Client locale unavailable; overlay keeps its default language");
            None
        }
    }
}

type TileResult = Result<(usize, Arc<[u8]>), String>;

async fn attach_tiles(
    catalog: &mut Catalog,
    assets: &ChampionAssets,
    client: &dekan_lcu::client::LcuClient,
) -> usize {
    use futures_util::StreamExt;

    let wanted: Vec<(usize, String)> = catalog
        .skins
        .iter()
        .enumerate()
        .filter_map(|(index, skin)| {
            assets
                .skins
                .iter()
                .find(|s| s.id == skin.id)
                .and_then(|s| s.tile_path.clone())
                .map(|path| (index, path))
        })
        .collect();
    let results: Vec<TileResult> = futures_util::stream::iter(wanted)
        .map(|(index, path)| {
            let client = client.clone();
            async move {
                match client.get_asset_bytes(&path).await {
                    Ok(bytes) if !bytes.is_empty() => Ok((index, Arc::from(bytes))),
                    Ok(_) => Err(format!("{path} answered with no bytes")),
                    Err(e) => Err(format!("{path}: {e}")),
                }
            }
        })
        .buffer_unordered(PREVIEW_FETCHES)
        .collect()
        .await;
    let mut fetched = 0usize;
    let mut failed = 0usize;
    let mut first_error: Option<String> = None;
    for result in results {
        match result {
            Ok((index, tile)) => {
                catalog.skins[index].tile = Some(tile);
                fetched += 1;
            }
            Err(e) => {
                failed += 1;
                first_error.get_or_insert(e);
            }
        }
    }
    if let Some(first_error) = first_error {
        warn!(
            failed,
            fetched, first_error, "Some skin tiles could not be fetched from the client"
        );
    }
    fetched
}

#[must_use]
pub fn resolve_library_root(configured: &Path) -> PathBuf {
    configured.to_path_buf()
}

mod classic;
mod preview;

pub use classic::{build_classic_catalog, load_classic_catalog};
use preview::PREVIEW_FETCHES;
pub use preview::{PreviewFetches, chroma_preview_fetches};

#[cfg(test)]
mod tests;
