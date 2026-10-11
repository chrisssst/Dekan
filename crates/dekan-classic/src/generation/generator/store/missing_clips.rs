use super::*;

impl StandardChampion {
    pub(crate) fn skin_graph(&self, source: &[u8]) -> Result<Option<(String, u32)>, ClassicError> {
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

    pub(crate) fn missing_clips(
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

    pub(crate) fn with_missing_clips(
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
}
