use super::*;

type RepairedProp = (Vec<u8>, Vec<Relink>);

pub struct Repairer<'a> {
    game: &'a BTreeMap<String, GameWad>,
    game_hashes: &'a HashSet<u64>,
    pools: HashMap<String, BTreeSet<String>>,
    opened: HashMap<PathBuf, Option<WadFile>>,
    game_formats: HashMap<String, Option<BTreeSet<AssetFormat>>>,
    unread: usize,
    first_unread: Option<String>,
}

impl<'a> Repairer<'a> {
    #[must_use]
    pub fn new(game: &'a BTreeMap<String, GameWad>, game_hashes: &'a HashSet<u64>) -> Self {
        Self {
            game,
            game_hashes,
            pools: HashMap::new(),
            opened: HashMap::new(),
            game_formats: HashMap::new(),
            unread: 0,
            first_unread: None,
        }
    }

    fn unread(&mut self, what: String) {
        self.unread += 1;
        self.first_unread.get_or_insert(what);
    }

    fn report_unread(&mut self, wad: &Path, before: usize) {
        if self.unread > before {
            warn!(
                wad = %wad.display(),
                unread = self.unread - before,
                first_error = self.first_unread.take().unwrap_or_default(),
                "Game or mod data could not be read while checking this mod; the check is partial"
            );
        }
    }

    fn game_shared_bins(&mut self, folder: &str) -> BTreeSet<String> {
        let mut names = BTreeSet::new();
        for slot in 0..SKIN_SLOTS_SCANNED {
            let hash = wad_path_hash(&format!("{folder}/skins/skin{slot}.bin"));
            let Some(holder) = self.game.values().find(|wad| wad.contains(hash)) else {
                continue;
            };
            if !self.opened.contains_key(&holder.path) {
                let opened = WadFile::open(&holder.path)
                    .inspect_err(|e| self.unread(format!("{}: {e}", holder.path.display())))
                    .ok();
                self.opened.insert(holder.path.clone(), opened);
            }
            let Some(Some(wad)) = self.opened.get(&holder.path) else {
                continue;
            };
            let links = match wad.read(hash) {
                Ok(Some(bytes)) => parse_prop_links(&bytes).map_err(|e| e.to_string()),
                Ok(None) => Err("missing from its own table".to_owned()),
                Err(e) => Err(e.to_string()),
            };
            match links {
                Ok(links) => names.extend(links.into_iter().filter(|l| shared_bin(l).is_some())),
                Err(e) => self.unread(format!("{folder}/skins/skin{slot}.bin: {e}")),
            }
        }
        names
    }

    fn formats_of_game_wad(&mut self, mount: &str) -> Option<BTreeSet<AssetFormat>> {
        if let Some(known) = self.game_formats.get(mount) {
            return known.clone();
        }
        let holder = self.game.values().find(|wad| {
            wad.relpath
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| mount_name(n) == mount)
        });
        let file = match holder.filter(|wad| wad.names.len() <= FORMAT_REFERENCE_MAX_ENTRIES) {
            Some(wad) => WadFile::open(&wad.path)
                .inspect_err(|e| self.unread(format!("{}: {e}", wad.path.display())))
                .ok(),
            None => {
                debug!(
                    mount,
                    "No game WAD small enough to read as the asset format reference; formats not checked"
                );
                None
            }
        };
        let known = file.map(|file| {
            let mut entries: Vec<(usize, u64)> =
                file.toc().map(|e| (e.offset, e.path_hash)).collect();
            entries.sort_unstable();
            let mut formats = BTreeSet::new();
            for (_, hash) in entries {
                match file.read_prefix(hash, FORMAT_PREFIX) {
                    Ok(Some(head)) => formats.extend(asset_format(&head)),
                    Ok(None) => {}
                    Err(e) => self.unread(format!("{hash:016x}: {e}")),
                }
            }
            formats
        });
        self.game_formats.insert(mount.to_owned(), known.clone());
        known
    }

    pub fn unknown_formats(&mut self, wad: &Path) -> Result<BTreeSet<AssetFormat>, InjectError> {
        let before = self.unread;
        let found = self.formats_unknown_to_game(wad);
        self.report_unread(wad, before);
        found
    }

    fn formats_unknown_to_game(
        &mut self,
        wad: &Path,
    ) -> Result<BTreeSet<AssetFormat>, InjectError> {
        let name = wad
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let Some(known) = self.formats_of_game_wad(&mount_name(&name)) else {
            return Ok(BTreeSet::new());
        };
        let mut heads: Vec<Vec<u8>> = Vec::new();
        if wad.is_dir() {
            let files = files_under(wad).map_err(|e| {
                InjectError::Overlay(format!("mod folder unreadable '{}': {e}", wad.display()))
            })?;
            for (_, path) in files {
                let mut head = vec![0u8; FORMAT_PREFIX];
                match std::fs::File::open(&path)
                    .and_then(|mut file| std::io::Read::read(&mut file, &mut head))
                {
                    Ok(read) => {
                        head.truncate(read);
                        heads.push(head);
                    }
                    Err(e) => self.unread(format!("{}: {e}", path.display())),
                }
            }
        } else {
            let file = WadFile::open(wad).map_err(|e| {
                InjectError::Overlay(format!("mod WAD unreadable '{}': {e}", wad.display()))
            })?;
            for entry in file.toc() {
                match file.read_prefix(entry.path_hash, FORMAT_PREFIX) {
                    Ok(Some(head)) => heads.push(head),
                    Ok(None) => {}
                    Err(e) => self.unread(format!("{:016x}: {e}", entry.path_hash)),
                }
            }
        }
        Ok(heads
            .iter()
            .filter_map(|head| asset_format(head))
            .filter(|format| known.iter().any(|k| k.kind == format.kind) && !known.contains(format))
            .collect())
    }

    fn repair_prop(
        &mut self,
        bytes: &[u8],
        own: &HashSet<u64>,
    ) -> Result<Option<RepairedProp>, dekan_wad::error::WadError> {
        if !bytes.starts_with(b"PROP") {
            return Ok(None);
        }
        let mut prop = match parse_prop_file(bytes) {
            Ok(prop) => prop,
            Err(e) => {
                debug!(error = %e, "Mod bin not relinked: it does not parse");
                return Ok(None);
            }
        };
        if !serialize_prop_file(&prop).is_ok_and(|again| again == bytes) {
            debug!("Mod bin not relinked: it does not round-trip byte for byte");
            return Ok(None);
        }
        let game_hashes = self.game_hashes;
        let in_game = |path: &str| game_hashes.contains(&wad_path_hash(path));
        let resolvable = |path: &str| {
            let hash = wad_path_hash(path);
            game_hashes.contains(&hash) || own.contains(&hash)
        };

        let known: BTreeSet<u32> = prop
            .links
            .iter()
            .filter_map(|l| shared_bin(l))
            .flat_map(|b| b.slots)
            .collect();
        let mut found = Vec::new();
        for (at, link) in prop.links.iter().enumerate() {
            if !is_bin_link(link) || resolvable(link) {
                continue;
            }
            let Some(old) = shared_bin(link) else {
                debug!(link = %link, "Mod bin not relinked: the missing link is not a shared skin bin");
                return Ok(None);
            };
            if !self.pools.contains_key(&old.folder) {
                let pool = self.game_shared_bins(&old.folder);
                self.pools.insert(old.folder.clone(), pool);
            }
            let Some(new) = self
                .pools
                .get(&old.folder)
                .and_then(|pool| successor(&old, &known, pool))
            else {
                debug!(link = %link, "Mod bin not relinked: the game has no single successor for this link");
                return Ok(None);
            };
            found.push((at, new.clone()));
        }
        let mut targets = BTreeSet::new();
        let clashes = found.iter().any(|(at, new)| {
            !targets.insert(new.clone())
                || prop
                    .links
                    .iter()
                    .enumerate()
                    .any(|(other, link)| other != *at && link.eq_ignore_ascii_case(new))
        });
        if clashes {
            debug!("Mod bin not relinked: two links would point at the same file");
            return Ok(None);
        }

        let mut relinks = Vec::new();
        for (at, new) in found {
            relinks.push(Relink {
                from: std::mem::replace(&mut prop.links[at], new.clone()),
                to: new,
            });
        }
        for entry in &mut prop.entries {
            let Ok(mut fields) = parse_fields(&entry.body) else {
                continue;
            };
            if !write_fields(&fields).is_ok_and(|again| again == entry.body) {
                continue;
            }
            let before = relinks.len();
            for field in &mut fields {
                relink_assets(&mut field.value, &resolvable, &in_game, &mut relinks);
            }
            if relinks.len() > before {
                entry.body = write_fields(&fields)?;
            }
        }
        if relinks.is_empty() {
            return Ok(None);
        }
        Ok(Some((serialize_prop_file(&prop)?, relinks)))
    }

    pub fn repair(&mut self, wad: &Path, mod_hashes: &HashSet<u64>) -> Result<Repair, InjectError> {
        let before = self.unread;
        let repaired = self.repair_wad(wad, mod_hashes);
        self.report_unread(wad, before);
        repaired
    }

    fn repair_wad(&mut self, wad: &Path, mod_hashes: &HashSet<u64>) -> Result<Repair, InjectError> {
        let overlay = |e: dekan_wad::error::WadError| {
            InjectError::Overlay(format!("repair of '{}': {e}", wad.display()))
        };
        let mut repair = Repair::default();

        if wad.is_dir() {
            let files = files_under(wad).map_err(|e| {
                InjectError::Overlay(format!("mod folder unreadable '{}': {e}", wad.display()))
            })?;
            let mut own: HashSet<u64> = files.iter().map(|(r, _)| relative_path_hash(r)).collect();
            own.extend(mod_hashes);
            for (relative, path) in files {
                if !ends_with_ci(&relative, ".bin") {
                    continue;
                }
                let bytes = std::fs::read(&path).map_err(|e| {
                    InjectError::Overlay(format!("repair of '{}': {e}", path.display()))
                })?;
                if let Some((fixed, relinks)) = self.repair_prop(&bytes, &own).map_err(overlay)? {
                    dekan_platform::fs::atomic_write(&path, &fixed, true).map_err(|e| {
                        InjectError::Overlay(format!("repair of '{}': {e}", path.display()))
                    })?;
                    repair.relinks.extend(relinks);
                    repair.files.push(path);
                }
            }
            return Ok(repair);
        }

        let file = WadFile::open(wad).map_err(overlay)?;
        let mut own: HashSet<u64> = file.toc().map(|e| e.path_hash).collect();
        own.extend(mod_hashes);
        let mut replaced = Vec::new();
        let hashes: Vec<u64> = file.toc().map(|e| e.path_hash).collect();
        for hash in hashes {
            let bytes = match starts_as_prop(&file, hash)
                .and_then(|prop| if prop { file.read(hash) } else { Ok(None) })
            {
                Ok(Some(bytes)) => bytes,
                Ok(None) => continue,
                Err(e) => {
                    self.unread(format!("{hash:016x}: {e}"));
                    continue;
                }
            };
            if let Some((fixed, relinks)) = self.repair_prop(&bytes, &own).map_err(overlay)? {
                repair.relinks.extend(relinks);
                replaced.push((hash, fixed));
            }
        }
        if replaced.is_empty() {
            return Ok(repair);
        }

        let mut writer = WadWriter::new(*file.signature());
        let source = writer.add_source(wad);
        for entry in file.toc() {
            writer.insert(entry.path_hash, WriterEntry::from_wad(source, entry));
        }
        for (hash, bytes) in replaced {
            writer.insert(hash, optimal_raw(bytes).map_err(overlay)?);
        }
        let rewritten = wad.with_extension("relinked");
        let written = writer.write_to_file(&rewritten, &|| false).map_err(overlay);
        drop(file);
        if let Err(e) = written.and_then(|_| {
            std::fs::rename(&rewritten, wad).map_err(|e| {
                InjectError::Overlay(format!("could not replace '{}': {e}", wad.display()))
            })
        }) {
            let _ = std::fs::remove_file(&rewritten); // ignore-ok: the write or rename error is what gets reported
            return Err(e);
        }
        repair.files.push(wad.to_path_buf());
        Ok(repair)
    }
}
