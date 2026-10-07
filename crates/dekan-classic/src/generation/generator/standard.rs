use super::*;

pub const STANDARD_MOD_PREFIX: &str = "std_";

pub(crate) type SharedScan = std::sync::Arc<std::sync::OnceLock<BTreeSet<String>>>;

pub(crate) fn shared_scan(alias: &str, stamp: &str) -> SharedScan {
    static SCANS: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, SharedScan>>,
    > = std::sync::OnceLock::new();
    if stamp.is_empty() {
        return SharedScan::default();
    }
    let prefix = format!("{}|", alias.to_ascii_lowercase());
    let key = format!("{prefix}{stamp}");
    let scans = SCANS.get_or_init(Default::default);
    let mut map = match scans.lock() {
        Ok(map) => map,
        Err(poisoned) => poisoned.into_inner(),
    };
    map.retain(|k, _| !k.starts_with(&prefix) || *k == key);
    std::sync::Arc::clone(map.entry(key).or_default())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrewarmGate {
    Go,
    Wait,
    Stop,
}

pub(crate) const PREWARM_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

#[must_use]
pub fn champion_aliases(game_dir: &Path) -> Vec<String> {
    let dir = game_dir.join("DATA").join("FINAL").join("Champions");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut aliases: Vec<String> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let alias = name.strip_suffix(".wad.client")?;
            (!alias.contains('.') && is_safe_alias(alias)).then(|| alias.to_owned())
        })
        .collect();
    aliases.sort_unstable();
    aliases
}

#[must_use]
pub fn companion_cache_is_current(game_dir: &Path, cache_dir: &Path, alias: &str) -> bool {
    let wad = game_dir
        .join("DATA")
        .join("FINAL")
        .join("Champions")
        .join(format!("{alias}.wad.client"));
    let stamp = wad_stamp(&wad);
    let cache = cache_dir.join(format!(
        "companion_names_{}.json",
        alias.to_ascii_lowercase()
    ));
    !stamp.is_empty()
        && std::fs::read(cache)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<CharacterCache>(&bytes).ok())
            .is_some_and(|cached| cached.source == stamp)
}

pub fn prewarm_companions(game_dir: &Path, cache_dir: &Path, gate: impl Fn() -> PrewarmGate) {
    let started = std::time::Instant::now();
    let aliases = champion_aliases(game_dir);
    let mut indexed = 0usize;
    'champions: for alias in &aliases {
        loop {
            match gate() {
                PrewarmGate::Go => break,
                PrewarmGate::Wait => std::thread::sleep(PREWARM_WAIT),
                PrewarmGate::Stop => break 'champions,
            }
        }
        if companion_cache_is_current(game_dir, cache_dir, alias) {
            indexed += 1;
            continue;
        }
        match StandardChampion::open(game_dir, alias) {
            Ok(champion) => {
                champion.with_cache_dir(cache_dir).scanned_names(true);
                indexed += 1;
            }
            Err(e) => debug!(alias, error = %e, "Champion not indexed ahead of time"),
        }
    }
    info!(
        champions = aliases.len(),
        indexed,
        elapsed_s = started.elapsed().as_secs(),
        "Companion characters indexed ahead of champion select"
    );
}

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

    fn scanned_names(&self, ahead_of_time: bool) -> &BTreeSet<String> {
        let main = self.alias.to_ascii_lowercase();
        self.scanned.get_or_init(|| match &self.cache_dir {
            Some(dir) => cached_names(
                &dir.join(format!("companion_names_{main}.json")),
                &self.wad_stamp,
                &self.alias,
                ahead_of_time,
                || character_names_in_bins(&self.wad, &self.alias),
            ),
            None => character_names_in_bins(&self.wad, &self.alias),
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
        let bin = self.read_skin_bin(&main, skin).ok().flatten()?;
        let parent = slot_identity(&bin).ok()?.parent;
        (parent != 0 && parent != skin).then_some(parent)
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
        self.read_skin_bin(&main, skin)
            .ok()
            .flatten()
            .and_then(|bin| crate::forms::gear_keys(&bin).ok())
            .map_or(0, |keys| keys.len())
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

    fn bake_form(
        &self,
        source: &[u8],
        generated: Vec<u8>,
        form: u32,
    ) -> Result<Vec<u8>, ClassicError> {
        let keys = crate::forms::gear_keys(source)?;
        let key = *keys.get(form as usize).ok_or_else(|| {
            ClassicError::Bin(format!(
                "form {form} does not exist; the skin has {} forms",
                keys.len()
            ))
        })?;
        let gear = self.gear_body(source, key)?;
        let mut file = parse_prop_file(&generated).map_err(|e| ClassicError::Bin(e.to_string()))?;
        let submeshes = self.submeshes_of(&file, &gear)?;
        crate::forms::bake_form(
            &mut file,
            &crate::forms::GearForm {
                index: form,
                gear_body: &gear,
                submeshes: &submeshes,
            },
        )?;
        serialize_prop_file(&file).map_err(|e| ClassicError::Bin(e.to_string()))
    }

    fn gear_body(&self, source: &[u8], key: u32) -> Result<Vec<u8>, ClassicError> {
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

    fn form_cycle(
        &self,
        source: &[u8],
        source_path: &str,
        generated: &[u8],
    ) -> Result<Option<FormCycle>, ClassicError> {
        let keys = crate::forms::gear_keys(source)?;
        if keys.len() < 2 || self.gear_count(0) > 0 {
            return Ok(None);
        }
        let swaps = keys
            .iter()
            .map(|key| {
                self.gear_body(source, *key)
                    .and_then(|body| crate::gear_toggle::gear_swap(&body))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let Some(markers) = crate::gear_toggle::markers(&swaps) else {
            return Ok(None);
        };
        let parsed = parse_prop_file(source).map_err(|e| ClassicError::Bin(e.to_string()))?;
        let Some(skin) = parsed
            .entries
            .iter()
            .find(|e| e.class_hash == SKIN_DATA_CLASS)
        else {
            return Ok(None);
        };
        let graph = field_value(
            &skin.body,
            &[
                prop_key_hash("skinAnimationProperties"),
                prop_key_hash("animationGraphData"),
            ],
        )
        .map_err(|e| ClassicError::Bin(e.to_string()))?
        .and_then(|v| v.as_u32());
        let Some(graph) = graph else {
            return Ok(None);
        };
        if graph == prop_key_hash(&format!("Characters/{}/Animations/Skin0", self.alias)) {
            return Ok(None);
        }
        let mut toggled = None;
        for link in parsed
            .links
            .iter()
            .filter(|l| l.to_ascii_lowercase().contains("/animations/"))
        {
            let path = link.to_ascii_lowercase();
            let Some(bytes) = self.wad.read(wad_path_hash(&path))? else {
                continue;
            };
            if let Some(bytes) = crate::gear_toggle::add_toggle(&bytes, graph, &swaps)? {
                toggled = Some((path, bytes));
                break;
            }
        }
        let Some(graph_file) = toggled else {
            return Ok(None);
        };
        let mut files = vec![graph_file];
        let mut drivers = 0;
        let skin0 = match crate::gear_toggle::drive_by_parts(generated, &markers)? {
            Some((bytes, count)) => {
                drivers += count;
                bytes
            }
            None => generated.to_vec(),
        };
        let skin0 = crate::forms::strip_gear_indicators(&skin0)?;
        if let Some((bytes, count)) = crate::gear_toggle::drive_by_parts(source, &markers)? {
            drivers += count;
            files.push((source_path.to_owned(), bytes));
        }
        Ok(Some(FormCycle {
            files,
            skin0,
            forms: swaps.len(),
            drivers,
        }))
    }

    fn with_form_cycle(
        &self,
        source: &[u8],
        source_path: &str,
        skin: u32,
        generated: Vec<u8>,
        wad_root: &Path,
    ) -> Result<Vec<u8>, ClassicError> {
        match self.form_cycle(source, source_path, &generated) {
            Ok(Some(plan)) => {
                for (path, bytes) in &plan.files {
                    let target = wad_root.join(path);
                    if let Some(dir) = target.parent() {
                        std::fs::create_dir_all(dir)?;
                    }
                    std::fs::write(&target, bytes)?;
                }
                info!(
                    alias = %self.alias,
                    skin,
                    forms = plan.forms,
                    drivers = plan.drivers,
                    files = ?plan.files.iter().map(|(path, _)| path.as_str()).collect::<Vec<_>>(),
                    "Ctrl+5 cycles the skin's forms in game"
                );
                Ok(plan.skin0)
            }
            Ok(None) => {
                debug!(alias = %self.alias, skin, "No in-game form cycling for this skin");
                Ok(generated)
            }
            Err(e) => {
                warn!(
                    alias = %self.alias,
                    skin,
                    error = %e,
                    "In-game form cycling not added; the skin keeps its first form"
                );
                Ok(generated)
            }
        }
    }

    fn skin_graph(&self, source: &[u8]) -> Result<Option<(String, u32)>, ClassicError> {
        let parsed = parse_prop_file(source).map_err(|e| ClassicError::Bin(e.to_string()))?;
        let Some(skin) = parsed
            .entries
            .iter()
            .find(|e| e.class_hash == SKIN_DATA_CLASS)
        else {
            return Ok(None);
        };
        let graph = field_value(
            &skin.body,
            &[
                prop_key_hash("skinAnimationProperties"),
                prop_key_hash("animationGraphData"),
            ],
        )
        .map_err(|e| ClassicError::Bin(e.to_string()))?
        .and_then(|v| v.as_u32());
        let Some(graph) = graph else {
            return Ok(None);
        };
        for link in parsed
            .links
            .iter()
            .filter(|l| l.to_ascii_lowercase().contains("/animations/"))
        {
            let path = link.to_ascii_lowercase();
            let holds = self
                .wad
                .read(wad_path_hash(&path))?
                .and_then(|bytes| parse_prop_file(&bytes).ok())
                .is_some_and(|bin| bin.entries.iter().any(|e| e.key_hash == graph));
            if holds {
                return Ok(Some((path, graph)));
            }
        }
        Ok(None)
    }

    fn missing_clips(
        &self,
        source: &[u8],
        wad_root: &Path,
    ) -> Result<Option<(String, crate::clip_alias::AliasedGraph)>, ClassicError> {
        let base_key = prop_key_hash(&format!("Characters/{}/Animations/Skin0", self.alias));
        let Some((path, graph)) = self.skin_graph(source)? else {
            return Ok(None);
        };
        if graph == base_key {
            return Ok(None);
        }
        let main = self.alias.to_ascii_lowercase();
        let Some(base) = self.wad.read(wad_path_hash(&format!(
            "data/characters/{main}/animations/skin0.bin"
        )))?
        else {
            return Ok(None);
        };
        let current = match std::fs::read(wad_root.join(&path)) {
            Ok(bytes) => bytes,
            Err(_) => match self.wad.read(wad_path_hash(&path))? {
                Some(bytes) => bytes,
                None => return Ok(None),
            },
        };
        let spells = self
            .wad
            .read(wad_path_hash(&format!("data/characters/{main}/{main}.bin")))?
            .map(|record| crate::clip_alias::spell_names(&record))
            .unwrap_or_default();
        if spells.is_empty() {
            return Ok(None);
        }
        Ok(
            crate::clip_alias::alias_missing_clips(&current, graph, &base, base_key, &spells)?
                .map(|aliased| (path, aliased)),
        )
    }

    fn with_missing_clips(
        &self,
        source: &[u8],
        skin: u32,
        wad_root: &Path,
    ) -> Result<(), ClassicError> {
        match self.missing_clips(source, wad_root) {
            Ok(Some((path, (bytes, aliases)))) => {
                let target = wad_root.join(&path);
                if let Some(dir) = target.parent() {
                    std::fs::create_dir_all(dir)?;
                }
                std::fs::write(&target, bytes)?;
                info!(
                    alias = %self.alias,
                    skin,
                    graph = %path,
                    clips = ?aliases
                        .iter()
                        .map(|a| format!("{:08x}->{:08x} of {}", a.missing, a.variant, a.variants))
                        .collect::<Vec<_>>(),
                    "Clips the default skin's animations ask for now point at the skin's own version"
                );
                Ok(())
            }
            Ok(None) => Ok(()),
            Err(e) => {
                warn!(
                    alias = %self.alias,
                    skin,
                    error = %e,
                    "Missing animation clips not aliased; the skin keeps the game's graph"
                );
                Ok(())
            }
        }
    }

    fn submeshes_of(&self, file: &PropFile, gear: &[u8]) -> Result<Vec<String>, ClassicError> {
        let mesh_path = |body: &[u8], path: &[&str]| {
            let hashes: Vec<u32> = path.iter().map(|p| prop_key_hash(p)).collect();
            field_value(body, &hashes).ok().flatten().and_then(|v| {
                v.bytes
                    .get(2..)
                    .map(|b| String::from_utf8_lossy(b).into_owned())
            })
        };
        let skn =
            mesh_path(gear, &["mGearData", "skinMeshProperties", "simpleSkin"]).or_else(|| {
                file.entries
                    .iter()
                    .find(|e| e.class_hash == SKIN_DATA_CLASS)
                    .and_then(|skin| mesh_path(&skin.body, &["skinMeshProperties", "simpleSkin"]))
            });
        let Some(skn) = skn else {
            return Ok(Vec::new());
        };
        match self.wad.read(wad_path_hash(&skn.to_ascii_lowercase()))? {
            Some(bytes) => crate::forms::skn_submesh_names(&bytes),
            None => Ok(Vec::new()),
        }
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
        for companion in self.companions() {
            let Some(source_skin) = self.companion_source_skin(&companion, skin, base_skin) else {
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

        let meta = partial.join("META");
        std::fs::create_dir_all(&meta)?;
        let info = serde_json::json!({
            "Author": "Dekan",
            "Name": format!("{} skin {skin}", self.alias),
            "Version": "1.0",
            "Description": "Generated dynamically from installed game WAD",
        });
        std::fs::write(meta.join("info.json"), info.to_string())?;
        let manifest = serde_json::json!({
            "alias": self.alias,
            "skin": skin,
            "base_skin": base_skin,
            "form": form,
            "game_wad": self.wad_stamp,
            "generated": records,
        });
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

#[must_use]
pub fn resolve_alias_with_id(
    game_dir: &Path,
    client_alias: Option<&str>,
    champion_id: Option<u32>,
    library_champion_dir: &Path,
) -> Option<String> {
    let champions = game_dir.join("DATA").join("FINAL").join("Champions");
    if let Some(alias) = client_alias.filter(|a| is_safe_alias(a)) {
        if champions.join(format!("{alias}.wad.client")).is_file() {
            return Some(alias.to_owned());
        }
    }
    if let Some(installed) =
        champion_id.and_then(|id| crate::client_data::champion_alias(game_dir, id))
    {
        if is_safe_alias(&installed) && champions.join(format!("{installed}.wad.client")).is_file()
        {
            return Some(installed);
        }
    }
    resolve_alias(game_dir, client_alias, library_champion_dir)
}

#[must_use]
pub fn resolve_alias(
    game_dir: &Path,
    client_alias: Option<&str>,
    library_champion_dir: &Path,
) -> Option<String> {
    let champions = game_dir.join("DATA").join("FINAL").join("Champions");
    if let Some(alias) = client_alias.filter(|a| is_safe_alias(a)) {
        if champions.join(format!("{alias}.wad.client")).is_file() {
            return Some(alias.to_owned());
        }
        warn!(
            alias,
            "Client alias has no champion WAD; trying the skin library"
        );
    }

    let mut archives: Vec<PathBuf> = Vec::new();
    let mut stack = vec![library_champion_dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("fantome") || e.eq_ignore_ascii_case("zip"))
            {
                archives.push(path);
            }
        }
    }
    archives.sort();

    for archive in archives {
        let Ok(file) = std::fs::File::open(&archive) else {
            continue;
        };
        match dekan_wad::fantome::wad_names_in_archive(std::io::BufReader::new(file)) {
            Ok(names) if names.len() == 1 => {
                if let Some(alias) = names.into_iter().next().filter(|a| is_safe_alias(a)) {
                    return Some(alias);
                }
            }
            Ok(names) => debug!(
                archive = %archive.display(),
                wads = names.len(),
                "Archive does not target exactly one champion WAD"
            ),
            Err(e) => debug!(archive = %archive.display(), error = %e, "Archive unreadable"),
        }
    }
    None
}
