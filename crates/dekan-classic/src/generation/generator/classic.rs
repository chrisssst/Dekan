use super::*;

pub struct ClassicChampion {
    alias: String,
    wad: WadFile,

    wad_stamp: String,
    main: String,
    main_display: String,
}

impl ClassicChampion {
    pub fn open(game_dir: &Path, alias: &str) -> Result<Self, ClassicError> {
        if !is_safe_alias(alias) {
            return Err(ClassicError::InvalidAlias(alias.to_owned()));
        }
        let path = game_dir
            .join("DATA")
            .join("FINAL")
            .join("Champions")
            .join(format!("{alias}.wad.client"));
        if !path.is_file() {
            return Err(ClassicError::ChampionNotFound {
                alias: format!("{alias} (no {})", path.display()),
            });
        }
        Ok(Self {
            alias: alias.to_owned(),
            wad: WadFile::open(&path)?,
            wad_stamp: wad_stamp(&path),
            main: main_character(alias),
            main_display: format!("Jade_{alias}"),
        })
    }

    #[must_use]
    pub fn with_client_character(mut self, classic_alias: Option<&str>) -> Self {
        let Some(display) = classic_alias.filter(|name| is_safe_alias(name)) else {
            return self;
        };
        let name = display.to_ascii_lowercase();
        if name == self.main {
            return self;
        }
        if self.wad.contains(wad_path_hash(&character_bin(&name))) {
            info!(
                alias = %self.alias,
                derived = %self.main,
                client = %name,
                "Rift Classic character named by the client"
            );
            self.main = name;
            self.main_display = display.to_owned();
        } else {
            warn!(
                alias = %self.alias,
                client = %name,
                derived = %self.main,
                "The client's Rift Classic character is not in the champion archive; keeping the derived name"
            );
        }
        self
    }

    #[must_use]
    pub fn main_character(&self) -> &str {
        &self.main
    }

    #[must_use]
    pub fn jade_names_in_bins(&self) -> BTreeSet<String> {
        character_names_in_bins(&self.wad, &self.alias)
            .into_iter()
            .filter(|name| name.starts_with("jade_"))
            .collect()
    }

    #[must_use]
    pub fn jade_names_from_bins_cached(&self, cache_dir: &Path) -> BTreeSet<String> {
        let cache_path = cache_dir.join(format!(
            "classic_bin_names_{}.json",
            self.alias.to_ascii_lowercase()
        ));
        cached_names(&cache_path, &self.wad_stamp, &self.alias, false, || {
            self.jade_names_in_bins()
        })
    }

    #[must_use]
    pub fn present_characters(&self, known: &BTreeSet<String>) -> Vec<String> {
        let mut candidates: BTreeSet<String> = known.clone();
        candidates.insert(self.main.clone());
        candidates
            .into_iter()
            .filter(|c| self.wad.contains(wad_path_hash(&character_bin(c))))
            .collect()
    }

    #[must_use]
    pub fn has_skin(&self, character: &str, skin: u32) -> bool {
        self.wad.contains(wad_path_hash(&skin_bin(character, skin)))
    }

    #[must_use]
    pub fn skin_numbers(&self, character: &str, limit: u32) -> Vec<u32> {
        (0..limit)
            .filter(|n| self.has_skin(character, *n))
            .collect()
    }

    pub fn build_mod(
        &self,
        skin: u32,
        slots: &[u32],
        known_characters: &BTreeSet<String>,
        mods_dir: &Path,
    ) -> Result<String, ClassicError> {
        let main = self.main.clone();
        let present = self.present_characters(known_characters);
        if !present.contains(&main) {
            return Err(ClassicError::ChampionNotFound {
                alias: format!("{} has no Rift Classic version in this patch", self.alias),
            });
        }
        let targets: Vec<&String> = present.iter().filter(|c| self.has_skin(c, skin)).collect();
        if targets.is_empty() {
            return Err(ClassicError::SkinNotFound {
                champion_id: 0,
                skin_id: skin,
            });
        }

        let folder = format!(
            "{CLASSIC_MOD_PREFIX}{}_{skin}",
            self.alias.to_ascii_lowercase()
        );
        let final_dir = mods_dir.join(&folder);
        let partial = mods_dir.join(format!("{folder}.partial"));
        remove_if_present(&partial)?;

        let mut written = 0usize;
        for character in &targets {
            let display = if **character == main {
                self.main_display.clone()
            } else {
                (*character).clone()
            };
            let source = self
                .wad
                .read(wad_path_hash(&skin_bin(character, skin)))?
                .ok_or_else(|| ClassicError::Bin(format!("{character} skin{skin}.bin vanished")))?;

            let bins_dir = partial
                .join("WAD")
                .join(format!("{}.wad.client", self.alias))
                .join("data")
                .join("characters")
                .join(character.as_str())
                .join("skins");
            std::fs::create_dir_all(&bins_dir)?;
            for slot in slots.iter().copied().filter(|slot| *slot != skin) {
                let identity = identity_at(&self.wad, character, slot);
                let bin = retarget_skin_bin(&source, &display, skin, slot, identity)?;
                std::fs::write(bins_dir.join(format!("skin{slot}.bin")), bin)?;
                written += 1;
            }

            let anim_target = animation_bin(character, skin);
            if self.wad.contains(wad_path_hash(&anim_target)) {
                if let Ok(Some(anim_source)) = self.wad.read(wad_path_hash(&anim_target)) {
                    let anim_dir = partial
                        .join("WAD")
                        .join(format!("{}.wad.client", self.alias))
                        .join("data")
                        .join("characters")
                        .join(character.as_str())
                        .join("animations");
                    let _ = std::fs::create_dir_all(&anim_dir); // ignore-ok: classic anim dir
                    for slot in slots.iter().copied().filter(|slot| *slot != skin) {
                        if let Ok(retargeted_anim) =
                            retarget_animation_bin(&anim_source, &display, skin, slot)
                        {
                            // ignore-ok: classic anim slot write
                            let _ = std::fs::write(
                                anim_dir.join(format!("skin{slot}.bin")),
                                retargeted_anim,
                            );
                        }
                    }
                }
            }
        }

        let meta = partial.join("META");
        std::fs::create_dir_all(&meta)?;
        let info = serde_json::json!({
            "Author": "Dekan",
            "Name": format!("{} skin {skin} (Rift Classic)", self.alias),
            "Version": "1.0",
            "Description": "Generated from installed game data",
        });
        std::fs::write(meta.join("info.json"), info.to_string())?;

        remove_if_present(&final_dir)?;
        std::fs::rename(&partial, &final_dir)?;

        info!(
            alias = %self.alias,
            skin,
            characters = ?targets,
            slots = ?slots,
            bins = written,
            folder = %folder,
            "Rift Classic mod generated from the installed game"
        );
        Ok(folder)
    }
}

#[must_use]
pub fn slots_for(client_skin_id: Option<u32>) -> Vec<u32> {
    let mut slots = CLASSIC_DEFAULT_SLOTS.to_vec();
    if let Some(current) = client_skin_id.map(skin_number) {
        if !slots.contains(&current) {
            slots.push(current);
        }
    }
    slots
}
