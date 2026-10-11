use super::*;

impl InjectionTrigger {
    pub(crate) async fn collect_mods(&self, key: ArmKey) -> Option<Vec<String>> {
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

    pub(crate) async fn party_mods(&self, key: ArmKey) -> Vec<String> {
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

    pub(crate) fn hash_table_path(&self) -> PathBuf {
        self.paths.tools_dir.join("hashes.game.txt")
    }

    pub(crate) async fn prepare_mods(&self, champ_id: u32, entry_id: u32) -> Option<Vec<String>> {
        self.prepare_package(champ_id, entry_id, true).await
    }
}

pub(crate) fn generation_options() -> dekan_classic::generator::GenerationOptions {
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

pub(crate) fn drop_incompatible_mods(
    staged: Vec<String>,
    mods_dir: &Path,
    game_dir: &Path,
) -> Vec<String> {
    let Ok(game) = dekan_inject::overlay_builder::get_or_index_game(game_dir) else {
        return staged;
    };
    let game_hashes = dekan_inject::mod_compat::game_hash_set(&game);
    let mut repairer = dekan_inject::mod_compat::Repairer::new(&game, &game_hashes);
    let mut kept = Vec::with_capacity(staged.len());
    for name in staged {
        let wads = dekan_inject::mod_compat::mod_wads(&mods_dir.join(&name));
        let own = dekan_inject::mod_compat::mod_hashes(&wads);
        let mut dangling = Vec::new();
        for wad in &wads {
            let found = match dekan_inject::mod_compat::check(wad, &game_hashes, &own) {
                Ok(compat) if compat.is_compatible() => continue,
                Ok(compat) => compat.dangling,
                Err(e) => {
                    debug!(mod_name = %name, error = %e, "Custom mod WAD not checked");
                    continue;
                }
            };
            match repairer.repair(wad, &own) {
                Ok(repair) if !repair.relinks.is_empty() => info!(
                    mod_name = %name,
                    wad = %wad.display(),
                    relinks = repair.relinks.len(),
                    "Custom mod relinked to the files of the installed patch"
                ),
                Ok(_) => {}
                Err(e) => warn!(
                    mod_name = %name,
                    error = %e,
                    "Custom mod could not be relinked to the installed patch"
                ),
            }
            match dekan_inject::mod_compat::check(wad, &game_hashes, &own) {
                Ok(compat) => dangling.extend(compat.dangling),
                Err(_) => dangling.extend(found),
            }
        }
        if dangling.is_empty() {
            kept.push(name);
        } else {
            warn!(
                mod_name = %name,
                dangling = ?dangling,
                "Custom mod is incompatible with the installed patch (its PROP links a .bin the \
                 game no longer has); dropped so it does not crash the game on the loading screen"
            );
        }
    }
    kept
}

pub(crate) fn find_skin_archive(
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

pub(crate) fn extracted_mod_is_complete(mod_dir: &Path) -> bool {
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

pub(crate) fn prepare_mod_directory(archive_path: &Path, target_dir: &Path) -> std::io::Result<()> {
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
