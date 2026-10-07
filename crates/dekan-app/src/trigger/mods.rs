use super::*;

impl InjectionTrigger {
    pub(super) async fn collect_mods(&self, key: ArmKey) -> Option<Vec<String>> {
        if is_classic(key.champ_id) {
            return self.classic_mods(key).await;
        }

        let mut mods = Vec::new();
        for (champ_id, entry_id) in key.picks() {
            match self.prepare_mods(champ_id, entry_id).await {
                Some(prepared) => mods.extend(prepared),
                None if !key.lobby => return None,
                None => warn!(
                    champ_id,
                    entry_id,
                    "The skin for this lobby champion could not be prepared; it will look stock"
                ),
            }
        }

        if key.mods != 0 {
            let (selection, champions) = {
                let state = self.state_rx.borrow();
                let champions: Vec<u32> = match state.lobby.as_ref().filter(|_| key.lobby) {
                    Some(lobby) => lobby.champions(),
                    None => vec![key.champ_id],
                };
                (state.mods.clone(), champions)
            };
            let current = if key.lobby {
                lobby_mods_fingerprint(&selection, &champions)
            } else {
                selection.fingerprint(Some(key.champ_id))
            };
            if current != key.mods {
                debug!(
                    champ_id = key.champ_id,
                    "Mod selection changed since the build was scheduled"
                );
            }
            let roots = self.paths.mod_roots.clone();
            let mods_dir = self.paths.mods_dir.clone();
            let game_dir = self.effective_game_dir(None);
            let staged = tokio::task::spawn_blocking(move || {
                let mut staged: Vec<String> = Vec::new();
                for champion in champions {
                    let champion = Some(champion);
                    let catalog = dekan_core::mods::scan_catalog(&roots, champion, &|_| true);
                    for name in dekan_app::mods_store::stage_selected(
                        &catalog, &selection, champion, &mods_dir,
                    ) {
                        if !staged.contains(&name) {
                            staged.push(name);
                        }
                    }
                }

                drop_incompatible_mods(staged, &mods_dir, &game_dir)
            })
            .await;
            match staged {
                Ok(staged) => mods.extend(staged),
                Err(e) => {
                    warn!(error = %e, "Custom mod staging task failed; continuing without them")
                }
            }
        }

        if key.party != 0 {
            mods.extend(self.party_mods(key).await);
        }

        if mods.is_empty() {
            warn!(
                champ_id = key.champ_id,
                "Nothing could be prepared for this build; no overlay will be made"
            );
            return None;
        }
        info!(
            champ_id = key.champ_id,
            mods = ?mods,
            "Mods prepared for the overlay build, in merge order"
        );
        Some(mods)
    }

    pub(super) async fn party_mods(&self, key: ArmKey) -> Vec<String> {
        let (accepted, rejected) = party_skins(&self.state_rx.borrow());
        for (member_id, reason) in &rejected {
            warn!(member_id, reason = ?reason, "Party skin not injected");
        }
        if dekan_core::party::party_fingerprint(&accepted) != key.party {
            debug!("Party skins changed since the build was scheduled; using the current ones");
        }

        let mut staged = Vec::new();
        for (champion_id, entry_id) in accepted {
            if is_classic(champion_id) {
                match self.prepare_classic_party_skin(champion_id, entry_id).await {
                    Some(names) => {
                        info!(champion_id, entry_id, "Classic party skin prepared");
                        staged.extend(names);
                    }
                    None => warn!(
                        champion_id,
                        entry_id, "Could not prepare classic party skin for teammate"
                    ),
                }
            } else {
                match self.prepare_package(champion_id, entry_id, false).await {
                    Some(names) => {
                        info!(champion_id, entry_id, "Party skin prepared");
                        staged.extend(names);
                    }
                    None => warn!(
                        champion_id,
                        entry_id,
                        "A teammate's skin is not in the local library; they will look stock to you"
                    ),
                }
            }
        }
        staged
    }

    pub(super) async fn prepare_classic_party_skin(
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

    pub(super) async fn classic_client_alias(&self, classic_id: u32) -> Option<String> {
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

    pub(super) async fn classic_mods(&self, key: ArmKey) -> Option<Vec<String>> {
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

    pub(super) fn hash_table_path(&self) -> PathBuf {
        self.paths.tools_dir.join("hashes.game.txt")
    }

    pub(super) async fn prepare_mods(&self, champ_id: u32, entry_id: u32) -> Option<Vec<String>> {
        self.prepare_package(champ_id, entry_id, true).await
    }

    pub(super) async fn prepare_package(
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

    pub(super) async fn prepare_dynamic_skin(
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

pub(super) fn generation_options() -> dekan_classic::generator::GenerationOptions {
    let set_to = |name: &str, value: &str| {
        std::env::var(name).is_ok_and(|v| v.trim().eq_ignore_ascii_case(value))
    };
    let options = dekan_classic::generator::GenerationOptions {
        graph_in_slot0: set_to(dekan_core::env::SKIN_GRAPH, "slot0"),
        chroma_keeps_classification: set_to(dekan_core::env::CHROMA_CLASSIFICATION, "source"),
    };
    if options != dekan_classic::generator::GenerationOptions::default() {
        info!(?options, "Skin generation test variant active");
    }
    options
}

pub(super) fn drop_incompatible_mods(
    staged: Vec<String>,
    mods_dir: &Path,
    game_dir: &Path,
) -> Vec<String> {
    let Ok(game) = dekan_inject::overlay_builder::get_or_index_game(game_dir) else {
        return staged;
    };
    let game_hashes = dekan_inject::mod_compat::game_hash_set(&game);
    staged
        .into_iter()
        .filter(|name| {
            let wad_dir = mods_dir.join(name).join("WAD");
            let Ok(entries) = std::fs::read_dir(&wad_dir) else {
                return true;
            };
            let mut dangling = Vec::new();
            for wad in entries.flatten().map(|e| e.path()) {
                if wad.extension().is_none_or(|ext| ext != "client") || !wad.is_file() {
                    continue;
                }
                if let Ok(compat) = dekan_inject::mod_compat::check_wad(&wad, &game_hashes) {
                    dangling.extend(compat.dangling);
                }
            }
            if dangling.is_empty() {
                true
            } else {
                warn!(
                    mod_name = %name,
                    dangling = ?dangling,
                    "Custom mod is incompatible with the installed patch (its PROP links a .bin the \
                     game no longer has); dropped so it does not crash the game on the loading screen"
                );
                false
            }
        })
        .collect()
}

pub(super) fn find_skin_archive(
    candidate_roots: &[PathBuf],
    champ_id: u32,
    skin_id: u32,
) -> Option<PathBuf> {
    for root in candidate_roots {
        if !root.is_dir() {
            continue;
        }

        let p1 = root
            .join(champ_id.to_string())
            .join(skin_id.to_string())
            .join(format!("{skin_id}.fantome"));
        if p1.is_file() {
            return Some(p1);
        }

        let p2 = root
            .join(champ_id.to_string())
            .join(format!("{skin_id}.fantome"));
        if p2.is_file() {
            return Some(p2);
        }

        let p3 = root.join(format!("{champ_id}_{skin_id}.fantome"));
        if p3.is_file() {
            return Some(p3);
        }

        let p4 = root.join(format!("{skin_id}.fantome"));
        if p4.is_file() {
            return Some(p4);
        }

        let p5 = root.join(champ_id.to_string()).join(skin_id.to_string());
        if p5.is_dir() {
            return Some(p5);
        }
    }
    None
}

pub(super) fn extracted_mod_is_complete(mod_dir: &Path) -> bool {
    let wad_dir = ["WAD", "wad"]
        .iter()
        .map(|name| mod_dir.join(name))
        .find(|path| path.is_dir());

    let Some(wad_dir) = wad_dir else {
        return false;
    };

    std::fs::read_dir(wad_dir)
        .map(|entries| entries.flatten().any(|entry| entry.path().is_file()))
        .unwrap_or(false)
}

pub(super) fn prepare_mod_directory(archive_path: &Path, target_dir: &Path) -> std::io::Result<()> {
    if target_dir.exists() {
        if extracted_mod_is_complete(target_dir) {
            return Ok(());
        }

        warn!(
            mod_dir = %target_dir.display(),
            "Extracted mod directory has no WAD; discarding it and extracting again"
        );
        std::fs::remove_dir_all(target_dir)?;
    }

    if archive_path.is_dir() {
        dekan_platform::fs::mirror_tree(archive_path, target_dir).map_err(std::io::Error::other)?;
        return Ok(());
    }

    let file = std::fs::File::open(archive_path)?;
    dekan_platform::fs::safe_extract_zip(
        std::io::BufReader::new(file),
        target_dir,
        &dekan_platform::fs::ExtractLimits::default(),
    )
    .map_err(std::io::Error::other)?;
    Ok(())
}
