use super::form_models::MergedModels;
use super::*;

pub(crate) struct Animations(pub(crate) Vec<(String, Vec<u8>)>);

pub(crate) struct Markers {
    pub(crate) names: Vec<String>,
    pub(crate) keys: Vec<u32>,
}

impl StandardChampion {
    pub(crate) fn animations(&self, links: &[String]) -> Result<Animations, ClassicError> {
        let mut out = Vec::new();
        for link in links {
            let path = link.to_ascii_lowercase();
            if !path.contains("/animations/") {
                continue;
            }
            if let Some(bytes) = self.wad.read(wad_path_hash(&path))? {
                out.push((path, bytes));
            }
        }
        Ok(Animations(out))
    }

    pub(crate) fn cycle_mesh(
        &self,
        merged: Option<&MergedModels>,
        generated: &[u8],
    ) -> Result<Option<(String, Vec<u8>)>, ClassicError> {
        if let Some(m) = merged {
            return Ok(Some((m.mesh_path.to_ascii_lowercase(), m.mesh.clone())));
        }
        let Some(path) = crate::form_marker::mesh_text(generated, "simpleSkin")? else {
            debug!(alias = %self.alias, "Form cycle skipped: the skin names no mesh");
            return Ok(None);
        };
        let path = path.to_ascii_lowercase();
        let Some(skn) = self.wad.read(wad_path_hash(&path))? else {
            debug!(alias = %self.alias, mesh = %path, "Form cycle skipped: the mesh is not in the game");
            return Ok(None);
        };
        Ok(Some((path, skn)))
    }

    pub(crate) fn cycle_markers(
        &self,
        merged: Option<&MergedModels>,
        forms: usize,
        animations: &Animations,
    ) -> Result<Option<Markers>, ClassicError> {
        let Some(m) = merged else {
            let names = crate::form_marker::marker_names(forms);
            let markers = names.iter().map(|name| prop_key_hash(name)).collect();
            return Ok(Some(Markers {
                names,
                keys: markers,
            }));
        };
        let mut hidden = BTreeSet::new();
        for (_, bytes) in &animations.0 {
            hidden.extend(crate::form_mesh::hidden_by_clips(bytes)?);
        }
        match crate::form_mesh::marker_parts(&m.parts, |p: &str| m.part_size(p), &hidden) {
            Ok(keys) => Ok(Some(Markers {
                names: Vec::new(),
                keys,
            })),
            Err(e) => {
                warn!(alias = %self.alias, reason = %e, "Form cycle skipped: no part can mark a form");
                Ok(None)
            }
        }
    }

    pub(crate) fn cycle_graph(
        &self,
        source: &[u8],
        graph: u32,
        merged: Option<&MergedModels>,
        swaps: &[crate::gear_toggle::GearSwap],
        markers: &[u32],
        animations: &Animations,
    ) -> Result<Option<(String, Vec<u8>)>, ClassicError> {
        let situations = match merged {
            Some(_) => crate::form_transition::situation_keys(source)?,
            None => BTreeSet::new(),
        };
        for (path, bytes) in &animations.0 {
            let transitioned;
            let swaps = match merged {
                Some(m) => {
                    let transitions = crate::form_transition::transition_clips(
                        bytes,
                        graph,
                        &situations,
                        &m.tokens,
                    )?;
                    let mut with = swaps.to_vec();
                    for (swap, transition) in with.iter_mut().zip(transitions) {
                        swap.transition = transition;
                    }
                    transitioned = with;
                    transitioned.as_slice()
                }
                None => swaps,
            };
            if let Some((graph_bin, _)) = crate::form_graph::build_form_graph(
                bytes,
                graph,
                swaps,
                markers,
                merged.map(|m| m.joints),
            )? {
                let transitions = swaps.iter().filter(|s| s.transition.is_some()).count();
                debug!(alias = %self.alias, graph = %path, transitions, "Form graph built");
                return Ok(Some((path.clone(), graph_bin)));
            }
        }
        debug!(alias = %self.alias, "Form cycle skipped: no linked animation graph could take the cycle");
        Ok(None)
    }
}
