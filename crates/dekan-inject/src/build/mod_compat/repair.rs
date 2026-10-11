use super::*;

type RepairedProp = (Vec<u8>, Vec<Relink>);

pub struct Repairer<'a> {
    game: &'a BTreeMap<String, GameWad>,
    game_hashes: &'a HashSet<u64>,
    pools: HashMap<String, BTreeSet<String>>,
    opened: HashMap<PathBuf, Option<WadFile>>,
    game_formats: HashMap<String, Option<BTreeSet<AssetFormat>>>,
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
        }
    }

    fn game_shared_bins(&mut self, folder: &str) -> BTreeSet<String> {
        let mut names = BTreeSet::new();
        for slot in 0..SKIN_SLOTS_SCANNED {
            let hash = wad_path_hash(&format!("{folder}/skins/skin{slot}.bin"));
            let Some(holder) = self.game.values().find(|wad| wad.contains(hash)) else {
                continue;
            };
            let wad = self
                .opened
                .entry(holder.path.clone())
                .or_insert_with(|| WadFile::open(&holder.path).ok());
            let Some(links) = wad
                .as_ref()
                .and_then(|wad| wad.read(hash).ok().flatten())
                .and_then(|bytes| parse_prop_links(&bytes).ok())
            else {
                continue;
            };
            names.extend(links.into_iter().filter(|l| shared_bin(l).is_some()));
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
        let known = holder
            .filter(|wad| wad.names.len() <= FORMAT_REFERENCE_MAX_ENTRIES)
            .and_then(|wad| WadFile::open(&wad.path).ok())
            .map(|file| {
                let mut entries: Vec<(usize, u64)> =
                    file.toc().map(|e| (e.offset, e.path_hash)).collect();
                entries.sort_unstable();
                entries
                    .into_iter()
                    .filter_map(|(_, hash)| file.read_prefix(hash, FORMAT_PREFIX).ok().flatten())
                    .filter_map(|head| asset_format(&head))
                    .collect()
            });
        self.game_formats.insert(mount.to_owned(), known.clone());
        known
    }

    pub fn unknown_formats(&mut self, wad: &Path) -> Result<BTreeSet<AssetFormat>, InjectError> {
        let name = wad
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let Some(known) = self.formats_of_game_wad(&mount_name(&name)) else {
            return Ok(BTreeSet::new());
        };
        let heads: Vec<Vec<u8>> = if wad.is_dir() {
            files_under(wad)
                .into_iter()
                .filter_map(|(_, path)| {
                    let mut head = vec![0u8; FORMAT_PREFIX];
                    let read = std::io::Read::read(&mut std::fs::File::open(path).ok()?, &mut head)
                        .ok()?;
                    head.truncate(read);
                    Some(head)
                })
                .collect()
        } else {
            let file = WadFile::open(wad).map_err(|e| {
                InjectError::Overlay(format!("mod WAD unreadable '{}': {e}", wad.display()))
            })?;
            let hashes: Vec<u64> = file.toc().map(|e| e.path_hash).collect();
            hashes
                .into_iter()
                .filter_map(|hash| file.read_prefix(hash, FORMAT_PREFIX).ok().flatten())
                .collect()
        };
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
        let Ok(mut prop) = parse_prop_file(bytes) else {
            return Ok(None);
        };
        if !serialize_prop_file(&prop).is_ok_and(|again| again == bytes) {
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
        let overlay = |e: dekan_wad::error::WadError| {
            InjectError::Overlay(format!("repair of '{}': {e}", wad.display()))
        };
        let mut repair = Repair::default();

        if wad.is_dir() {
            let files = files_under(wad);
            let mut own: HashSet<u64> = files.iter().map(|(r, _)| relative_path_hash(r)).collect();
            own.extend(mod_hashes);
            for (relative, path) in files {
                if !ends_with_ci(&relative, ".bin") {
                    continue;
                }
                let bytes = std::fs::read(&path)?;
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
            if !starts_as_prop(&file, hash) {
                continue;
            }
            let Ok(Some(bytes)) = file.read(hash) else {
                continue;
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
        if let Err(e) =
            written.and_then(|_| std::fs::rename(&rewritten, wad).map_err(InjectError::from))
        {
            let _ = std::fs::remove_file(&rewritten); // ignore-ok: the write or rename error is what gets reported
            return Err(e);
        }
        repair.files.push(wad.to_path_buf());
        Ok(repair)
    }
}
