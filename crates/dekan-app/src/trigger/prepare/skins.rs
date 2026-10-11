use super::mods::{find_skin_archive, generation_options, prepare_mod_directory};
use super::*;

impl InjectionTrigger {
    pub(crate) async fn prepare_classic_party_skin(
        &self,
        champion_id: u32,
        entry_id: u32,
    ) -> Option<Vec<String>> {
        use dekan_classic::generator::{
            ClassicChampion, jade_characters, resolve_alias_with_id, skin_number, slots_for,
        };

        let regular = dekan_classic::builder::normalize_champion_id(champion_id);
        let skin = skin_number(entry_id);
        if skin == 0 {
            return None;
        }
        let slots = slots_for(None);

        let client_alias = match self.lcu_client().await {
            Some(client) => match client.get_champion_assets(regular).await {
                Ok(assets) if !assets.alias.is_empty() => Some(assets.alias),
                Ok(_) => None,
                Err(e) => {
                    debug!(error = %e, regular, "Teammate assets unavailable for Classic alias");
                    None
                }
            },
            None => None,
        };

        let game_dir = self.effective_game_dir(None);
        let library_dir = self.paths.library_dir.join(regular.to_string());
        let hashes = self.hash_table_path();
        let cache = self.paths.state_dir.join("classic_characters.json");
        let cache_dir = self.paths.state_dir.clone();
        let mods_dir = self.paths.mods_dir.clone();
        let classic_alias = self.classic_client_alias(champion_id).await;
        let classic_id = champion_id;
        let built = tokio::task::spawn_blocking(move || {
            let alias = resolve_alias_with_id(
                &game_dir,
                client_alias.as_deref(),
                Some(regular),
                &library_dir,
            )
            .ok_or_else(|| dekan_classic::error::ClassicError::ChampionNotFound {
                alias: format!("no WAD alias found for teammate champion {regular}"),
            })?;
            let classic_alias = classic_alias
                .or_else(|| dekan_classic::client_data::champion_alias(&game_dir, classic_id));
            let champion = ClassicChampion::open(&game_dir, &alias)?
                .with_client_character(classic_alias.as_deref());
            let mut known = jade_characters(&hashes, &cache);
            if known.is_empty() {
                known = champion.jade_names_from_bins_cached(&cache_dir);
            }
            champion.build_mod(skin, &slots, &known, &mods_dir)
        })
        .await;

        match built {
            Ok(Ok(folder)) => {
                info!(champion_id, regular, skin, %folder, "Teammate classic mod ready");
                Some(vec![folder])
            }
            Ok(Err(e)) => {
                warn!(champion_id, regular, skin, error = %e, "Could not build teammate classic mod");
                None
            }
            Err(e) => {
                error!(champion_id, error = %e, "Teammate classic build task failed");
                None
            }
        }
    }

    pub(crate) async fn classic_client_alias(&self, classic_id: u32) -> Option<String> {
        let client = self.lcu_client().await?;
        match client.get_champion_assets(classic_id).await {
            Ok(assets) if assets.alias.to_ascii_lowercase().starts_with("jade_") => {
                Some(assets.alias)
            }
            Ok(_) => None,
            Err(e) => {
                debug!(error = %e, classic_id, "Classic champion assets unavailable from the client");
                None
            }
        }
    }

    pub(crate) async fn classic_mods(&self, key: ArmKey) -> Option<Vec<String>> {
        use dekan_classic::generator::{
            ClassicChampion, jade_characters, resolve_alias_with_id, skin_number, slots_for,
        };

        let Some(entry_id) = key.entry_id else {
            debug!(
                champ_id = key.champ_id,
                "Rift Classic takes no custom mod; nothing to build"
            );
            return None;
        };
        let regular = dekan_classic::builder::normalize_champion_id(key.champ_id);
        let skin = skin_number(entry_id);
        let slots = slots_for(key.classic_slot);

        let client_alias = match self.lcu_client().await {
            Some(client) => match client.get_champion_assets(regular).await {
                Ok(assets) if !assets.alias.is_empty() => Some(assets.alias),
                Ok(_) => None,
                Err(e) => {
                    debug!(error = %e, regular, "Champion assets unavailable for the Classic alias");
                    None
                }
            },
            None => None,
        };

        let game_dir = self.effective_game_dir(None);
        let library_dir = self.paths.library_dir.join(regular.to_string());
        let hashes = self.hash_table_path();
        let cache = self.paths.state_dir.join("classic_characters.json");
        let cache_dir = self.paths.state_dir.clone();
        let mods_dir = self.paths.mods_dir.clone();
        let started = std::time::Instant::now();
        let classic_alias = self.classic_client_alias(key.champ_id).await;
        let classic_id = key.champ_id;
        let built = tokio::task::spawn_blocking(move || {
            let alias = resolve_alias_with_id(
                &game_dir,
                client_alias.as_deref(),
                Some(regular),
                &library_dir,
            )
            .ok_or_else(|| dekan_classic::error::ClassicError::ChampionNotFound {
                alias: format!("no WAD alias found for champion {regular}"),
            })?;
            let classic_alias = classic_alias
                .or_else(|| dekan_classic::client_data::champion_alias(&game_dir, classic_id));
            let champion = ClassicChampion::open(&game_dir, &alias)?
                .with_client_character(classic_alias.as_deref());

            let mut known = jade_characters(&hashes, &cache);
            if known.is_empty() {
                known = champion.jade_names_from_bins_cached(&cache_dir);
            }
            champion.build_mod(skin, &slots, &known, &mods_dir)
        })
        .await;

        match built {
            Ok(Ok(folder)) => {
                info!(
                    champ_id = key.champ_id,
                    regular,
                    skin,
                    slots = ?slots_for(key.classic_slot),
                    elapsed_ms = started.elapsed().as_millis(),
                    folder = %folder,
                    "Rift Classic mod ready"
                );
                Some(vec![folder])
            }
            Ok(Err(e)) => {
                warn!(
                    champ_id = key.champ_id,
                    regular,
                    skin,
                    error = %e,
                    "Rift Classic skin cannot be shown; nothing will be injected"
                );
                set_injection_status(
                    &self.state_tx,
                    InjectionStatus::Failed {
                        error: format!("Rift Classic: {e}"),
                    },
                );
                None
            }
            Err(e) => {
                error!(error = %e, "Rift Classic generation task failed");
                None
            }
        }
    }

    pub(crate) async fn prepare_package(
        &self,
        champ_id: u32,
        entry_id: u32,
        report_failure: bool,
    ) -> Option<Vec<String>> {
        let library_root = self.paths.library_dir.clone();
        let scan = tokio::task::spawn_blocking(move || {
            dekan_core::library::scan_champion(&library_root, champ_id)
        })
        .await;

        let archive_path = match &scan {
            Ok(library) => library.package_for(entry_id).map(Path::to_path_buf),
            Err(e) => {
                warn!(error = %e, "Library scan task failed; falling back to path probing");
                None
            }
        }
        .or_else(|| {
            find_skin_archive(
                std::slice::from_ref(&self.paths.library_dir),
                champ_id,
                entry_id,
            )
        });

        let Some(archive_path) = archive_path else {
            return self
                .prepare_dynamic_skin(champ_id, entry_id, report_failure)
                .await;
        };

        let mod_name = format!("{champ_id}_{entry_id}");
        let target_dir = self.paths.mods_dir.join(&mod_name);

        info!(
            archive = %archive_path.display(),
            mod_name = %mod_name,
            "Found the mod package for the chosen entry; preparing the mod directory"
        );

        if let Err(e) = prepare_mod_directory(&archive_path, &target_dir) {
            error!(error = %e, mod_name = %mod_name, "Could not prepare the mod directory");
            if report_failure {
                set_injection_status(
                    &self.state_tx,
                    InjectionStatus::Failed {
                        error: format!("mod package could not be extracted: {e}"),
                    },
                );
            }
            return None;
        }

        Some(vec![mod_name])
    }

    pub(crate) async fn prepare_dynamic_skin(
        &self,
        champ_id: u32,
        entry_id: u32,
        report_failure: bool,
    ) -> Option<Vec<String>> {
        use dekan_classic::generator::{StandardChampion, resolve_alias_with_id, skin_number};

        let skin = skin_number(entry_id);
        if skin == 0 {
            debug!(champ_id, entry_id, "Base skin selected; nothing to inject");
            return None;
        }

        let game_dir = self.effective_game_dir(None);
        if !game_dir.is_dir() {
            warn!(
                champ_id,
                entry_id, "Game directory not found; cannot dynamically generate skin"
            );
            if report_failure {
                set_injection_status(
                    &self.state_tx,
                    InjectionStatus::Failed {
                        error: "Game directory not found".into(),
                    },
                );
            }
            return None;
        }

        let assets = match self.lcu_client().await {
            Some(client) => match client.get_champion_assets(champ_id).await {
                Ok(assets) => Some(assets),
                Err(e) => {
                    debug!(
                        error = %e,
                        champ_id,
                        "Champion assets unavailable for dynamic skin generation"
                    );
                    None
                }
            },
            None => None,
        };
        let client_alias = assets
            .as_ref()
            .map(|a| a.alias.clone())
            .filter(|alias| !alias.is_empty());
        let base_skin = assets
            .as_ref()
            .and_then(|a| a.base_skin_of(entry_id))
            .map(skin_number);

        let library_dir = self.paths.library_dir.join(champ_id.to_string());
        let mods_dir = self.paths.mods_dir.clone();
        let cache_dir = self.paths.state_dir.clone();
        let built = tokio::task::spawn_blocking(move || {
            let alias = resolve_alias_with_id(
                &game_dir,
                client_alias.as_deref(),
                Some(champ_id),
                &library_dir,
            )
            .ok_or_else(|| dekan_classic::error::ClassicError::ChampionNotFound {
                alias: format!("no WAD alias found for champion {champ_id}"),
            })?;
            let champion = StandardChampion::open(&game_dir, &alias)?
                .with_cache_dir(&cache_dir)
                .with_options(generation_options());
            champion.build_mod(skin, base_skin, &mods_dir)
        })
        .await;

        match built {
            Ok(Ok(folder)) => {
                info!(
                    champ_id,
                    entry_id,
                    folder = %folder,
                    "Dynamic skin mod generated directly from installed game WAD"
                );
                Some(vec![folder])
            }
            Ok(Err(e)) => {
                warn!(
                    champ_id,
                    entry_id,
                    error = %e,
                    "Could not generate dynamic skin from installed game WAD"
                );
                if report_failure {
                    set_injection_status(
                        &self.state_tx,
                        InjectionStatus::Failed {
                            error: format!("Dynamic skin generation failed: {e}"),
                        },
                    );
                }
                None
            }
            Err(e) => {
                error!(champ_id, error = %e, "Dynamic skin generation task failed");
                None
            }
        }
    }
}
