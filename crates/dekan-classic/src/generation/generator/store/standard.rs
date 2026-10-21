use super::*;

pub const STANDARD_MOD_PREFIX: &str = "std_";

#[derive(Debug)]
pub struct StandardChampion {
    pub alias: String,
    pub(crate) wad: WadFile,
    wad_stamp: String,
    cache_dir: Option<PathBuf>,
    scanned: SharedScan,
    options: GenerationOptions,
}

impl StandardChampion {
    pub fn open(game_dir: &Path, alias: &str) -> Result<Self, ClassicError> {
        if !is_safe_alias(alias) {
            return Err(ClassicError::InvalidAlias(alias.to_owned()));
        }
        let wad_path = game_dir
            .join("DATA")
            .join("FINAL")
            .join("Champions")
            .join(format!("{alias}.wad.client"));
        let wad = WadFile::open(&wad_path).map_err(ClassicError::Wad)?;
        let stamp = wad_stamp(&wad_path);
        Ok(Self {
            alias: alias.to_owned(),
            wad,
            scanned: shared_scan(alias, &stamp),
            wad_stamp: stamp,
            cache_dir: None,
            options: GenerationOptions::default(),
        })
    }

    #[must_use]
    pub fn with_options(mut self, options: GenerationOptions) -> Self {
        self.options = options;
        self
    }

    fn slot0_identity(&self, character: &str) -> Option<SlotIdentity> {
        identity_at(&self.wad, character, 0).map(|identity| SlotIdentity {
            classification: identity
                .classification
                .filter(|_| !self.options.chroma_keeps_classification),
            ..identity
        })
    }

    fn finish_slot0(
        &self,
        character: &str,
        display: &str,
        source: &[u8],
        generated: Vec<u8>,
        characters_dir: &Path,
    ) -> Result<Vec<u8>, ClassicError> {
        if !self.options.graph_in_slot0 {
            return Ok(generated);
        }
        let (skin_bin, graph) = move_graph_to_slot0(&self.wad, display, source, generated)?;
        if let Some(graph) = graph {
            let dir = characters_dir.join(character).join("animations");
            std::fs::create_dir_all(&dir)?;
            std::fs::write(dir.join("skin0.bin"), graph)?;
            info!(character, "Animation graph moved to slot 0 (test variant)");
        }
        Ok(skin_bin)
    }

    #[must_use]
    pub fn with_cache_dir(mut self, cache_dir: &Path) -> Self {
        self.cache_dir = Some(cache_dir.to_path_buf());
        self
    }

    #[must_use]
    pub fn has_skin(&self, skin: u32) -> bool {
        let main = self.alias.to_ascii_lowercase();
        self.wad.contains(wad_path_hash(&skin_bin(&main, skin)))
    }

    #[must_use]
    pub fn skin_numbers(&self, limit: u32) -> Vec<u32> {
        (0..limit).filter(|n| self.has_skin(*n)).collect()
    }

    pub(crate) fn scanned_names(&self, ahead_of_time: bool) -> &BTreeSet<String> {
        let main = self.alias.to_ascii_lowercase();
        self.scanned.get_or_init(|| match &self.cache_dir {
            Some(dir) => cached_names(
                &dir.join(format!("companion_names_{main}.json")),
                &self.wad_stamp,
                &self.alias,
                ahead_of_time,
                || character_names_in_bins(&self.wad, &self.alias),
            ),
            None => character_names_in_bins(&self.wad, &self.alias).0,
        })
    }

    #[must_use]
    pub fn companions(&self) -> BTreeSet<String> {
        let main = self.alias.to_ascii_lowercase();
        let mut names: BTreeSet<String> = self.scanned_names(false).clone();
        names.remove(&main);
        names.retain(|name| is_safe_alias(name) && !name.starts_with("jade_"));
        names
    }

    #[must_use]
    pub fn companion_source_skin(
        &self,
        companion: &str,
        skin: u32,
        base_skin: Option<u32>,
    ) -> Option<u32> {
        std::iter::once(skin)
            .chain(base_skin.filter(|base| *base != skin && *base != 0))
            .find(|n| self.wad.contains(wad_path_hash(&skin_bin(companion, *n))))
    }

    #[must_use]
    pub fn parent_skin(&self, skin: u32) -> Option<u32> {
        let main = self.alias.to_ascii_lowercase();
        let bin = match self.read_skin_bin(&main, skin) {
            Ok(Some(bin)) => bin,
            Ok(None) => return None,
            Err(e) => {
                warn!(alias = %self.alias, skin, error = %e, "Skin bin unreadable; the client's parent skin is used");
                return None;
            }
        };
        match slot_identity(&bin) {
            Ok(identity) => {
                (identity.parent != 0 && identity.parent != skin).then_some(identity.parent)
            }
            Err(e) => {
                warn!(alias = %self.alias, skin, error = %e, "Skin identity unreadable; the client's parent skin is used");
                None
            }
        }
    }

    #[must_use]
    pub fn contains_path(&self, path: &str) -> bool {
        self.wad.contains(wad_path_hash(&path.to_ascii_lowercase()))
    }

    pub fn read_skin_bin(
        &self,
        character: &str,
        skin: u32,
    ) -> Result<Option<Vec<u8>>, ClassicError> {
        Ok(self.wad.read(wad_path_hash(&skin_bin(character, skin)))?)
    }

    pub fn gear_count(&self, skin: u32) -> usize {
        let main = self.alias.to_ascii_lowercase();
        let keys = match self.read_skin_bin(&main, skin) {
            Ok(Some(bin)) => crate::forms::gear_keys(&bin).map_err(|e| e.to_string()),
            Ok(None) => return 0,
            Err(e) => Err(e.to_string()),
        };
        match keys {
            Ok(keys) => keys.len(),
            Err(error) => {
                warn!(alias = %self.alias, skin, error = %error, "Skin gears unreadable; the skin is treated as having no forms");
                0
            }
        }
    }

    pub fn build_mod(
        &self,
        skin: u32,
        base_skin: Option<u32>,
        mods_dir: &Path,
    ) -> Result<String, ClassicError> {
        self.build(skin, base_skin, None, mods_dir)
    }

    pub fn build_mod_form(
        &self,
        skin: u32,
        base_skin: Option<u32>,
        form: u32,
        mods_dir: &Path,
    ) -> Result<String, ClassicError> {
        self.build(skin, base_skin, Some(form), mods_dir)
    }

    pub(crate) fn gear_body(&self, source: &[u8], key: u32) -> Result<Vec<u8>, ClassicError> {
        let parsed = parse_prop_file(source).map_err(|e| ClassicError::Bin(e.to_string()))?;
        if let Some(entry) = parsed.entries.iter().find(|e| e.key_hash == key) {
            return Ok(entry.body.clone());
        }
        crate::forms::find_linked_object(&self.wad, &parsed.links, key)?.ok_or_else(|| {
            ClassicError::Bin(format!(
                "gear {key:08x} is neither in the skin's bin nor in the bins it links"
            ))
        })
    }

    fn build(
        &self,
        skin: u32,
        base_skin: Option<u32>,
        form: Option<u32>,
        mods_dir: &Path,
    ) -> Result<String, ClassicError> {
        let main = self.alias.to_ascii_lowercase();
        let target_bin = skin_bin(&main, skin);
        if !self.wad.contains(wad_path_hash(&target_bin)) {
            return Err(ClassicError::SkinNotFound {
                champion_id: 0,
                skin_id: skin,
            });
        }

        let game_parent = self.parent_skin(skin);
        if game_parent.is_some() && base_skin.is_some() && game_parent != base_skin {
            debug!(
                alias = %self.alias,
                skin,
                game_parent = ?game_parent,
                client_base = ?base_skin,
                "The game and the client name different parent skins; the game's is used"
            );
        }
        let base_skin = game_parent.or(base_skin);

        let folder = match form {
            Some(form) => format!("{STANDARD_MOD_PREFIX}{main}_{skin}_form{form}"),
            None => format!("{STANDARD_MOD_PREFIX}{main}_{skin}"),
        };
        let final_dir = mods_dir.join(&folder);
        let key = serde_json::json!({
            "alias": self.alias,
            "skin": skin,
            "base_skin": base_skin,
            "form": form,
            "game_wad": self.wad_stamp,
            "generator": reuse::generator_stamp(),
            "options": format!("{:?}", self.options),
        });
        let _claim = reuse::Claim::wait_for(&final_dir);
        if reuse::already_built(&final_dir, &key) {
            info!(
                alias = %self.alias,
                skin,
                folder = %folder,
                "Generated skin reused; the game archive and Dekan are unchanged"
            );
            return Ok(folder);
        }
        let partial = mods_dir.join(format!("{folder}.partial"));
        remove_if_present(&partial)?;
        let wad_root = partial
            .join("WAD")
            .join(format!("{}.wad.client", self.alias));
        let characters_dir = wad_root.join("data").join("characters");

        let source = self
            .wad
            .read(wad_path_hash(&target_bin))?
            .ok_or_else(|| ClassicError::Bin(format!("{main} skin{skin}.bin not found in WAD")))?;
        let bins_dir = characters_dir.join(&main).join("skins");
        std::fs::create_dir_all(&bins_dir)?;
        let retargeted =
            retarget_skin_bin(&source, &self.alias, skin, 0, self.slot0_identity(&main))?;
        let retargeted = match form {
            Some(form) => self.bake_form(&source, retargeted, form)?,
            None => retargeted,
        };
        let retargeted =
            self.finish_slot0(&main, &self.alias, &source, retargeted, &characters_dir)?;
        let retargeted = if form.is_none() && !self.options.graph_in_slot0 {
            let retargeted =
                self.with_form_cycle(&source, &target_bin, skin, retargeted, &wad_root)?;
            self.with_missing_clips(&source, skin, &wad_root)?;
            retargeted
        } else {
            retargeted
        };
        let mut records = vec![generated_bin_record(
            &self.alias,
            &main,
            skin,
            &source,
            &retargeted,
        )];
        std::fs::write(bins_dir.join("skin0.bin"), retargeted)?;

        let mut retargeted_companions = Vec::new();
        let mut without_skin = Vec::new();
        for companion in self.companions() {
            let Some(source_skin) = self.companion_source_skin(&companion, skin, base_skin) else {
                without_skin.push(companion);
                continue;
            };
            let written = self
                .read_skin_bin(&companion, source_skin)
                .and_then(|source| {
                    source.ok_or_else(|| {
                        ClassicError::Bin(format!("{companion} skin{source_skin}.bin vanished"))
                    })
                })
                .and_then(|source| {
                    let retargeted = retarget_skin_bin(
                        &source,
                        &companion,
                        source_skin,
                        0,
                        self.slot0_identity(&companion),
                    )?;
                    let retargeted = self.finish_slot0(
                        &companion,
                        &companion,
                        &source,
                        retargeted,
                        &characters_dir,
                    )?;
                    records.push(generated_bin_record(
                        &self.alias,
                        &companion,
                        source_skin,
                        &source,
                        &retargeted,
                    ));
                    Ok(retargeted)
                })
                .and_then(|retargeted| {
                    let comp_dir = characters_dir.join(&companion).join("skins");
                    std::fs::create_dir_all(&comp_dir)?;
                    std::fs::write(comp_dir.join("skin0.bin"), retargeted)?;
                    Ok(())
                });
            if let Err(e) = written {
                warn!(
                    alias = %self.alias,
                    companion = %companion,
                    skin = source_skin,
                    error = %e,
                    "Companion skin not generated; it keeps its base look in this match"
                );
                continue;
            }
            retargeted_companions.push(companion);
        }
        if !without_skin.is_empty() {
            debug!(alias = %self.alias, skin, companions = ?without_skin, "Companions with no skin of their own keep their base look");
        }

        let meta = partial.join("META");
        std::fs::create_dir_all(&meta)?;
        let info = serde_json::json!({
            "Author": "Dekan",
            "Name": format!("{} skin {skin}", self.alias),
            "Version": "1.0",
            "Description": "Generated dynamically from installed game WAD",
        });
        std::fs::write(meta.join("info.json"), info.to_string())?;
        let mut manifest = key;
        manifest["generated"] = serde_json::json!(records);
        std::fs::write(
            meta.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest).map_err(|e| ClassicError::Bin(e.to_string()))?,
        )?;

        remove_if_present(&final_dir)?;
        std::fs::rename(&partial, &final_dir)?;

        info!(
            alias = %self.alias,
            skin,
            companions = ?retargeted_companions,
            folder = %folder,
            "Standard skin mod generated directly from installed game WAD"
        );

        Ok(folder)
    }
}
