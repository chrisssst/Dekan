use super::form_models::ModelPlan;
use super::*;
use crate::gear_toggle::bin_error;

fn swaps_of(gear_bodies: &[Vec<u8>]) -> Result<Vec<crate::gear_toggle::GearSwap>, ClassicError> {
    gear_bodies
        .iter()
        .map(|body| crate::gear_toggle::gear_swap(body))
        .collect()
}

impl StandardChampion {
    pub fn form_coverage(
        &self,
        skin: u32,
    ) -> Result<Option<(usize, crate::form_gear::GearCoverage)>, ClassicError> {
        let Some(source) = self.wad.read(wad_path_hash(&skin_bin(
            &self.alias.to_ascii_lowercase(),
            skin,
        )))?
        else {
            return Ok(None);
        };
        let Some(super::form_models::FormGears {
            bodies: gear_bodies,
            plan,
        }) = self.form_gears(&source)?
        else {
            return Ok(None);
        };
        let (gear_bodies, model) = match plan {
            ModelPlan::Merged(m) => {
                let outcome = m.outcome();
                (m.gear_bodies, Some(outcome))
            }
            ModelPlan::Shared => (gear_bodies, None),
            ModelPlan::Refused(reason) => (
                gear_bodies,
                Some(crate::form_gear::ModelOutcome::Refused(reason)),
            ),
        };
        let swaps = swaps_of(&gear_bodies)?;
        let mut coverage = crate::form_gear::gear_coverage(&source, &gear_bodies, &swaps)?;
        coverage.model = model;
        Ok(Some((swaps.len(), coverage)))
    }

    fn form_cycle(
        &self,
        source: &[u8],
        source_path: &str,
        generated: &[u8],
    ) -> Result<Option<FormCycle>, ClassicError> {
        let Some(super::form_models::FormGears {
            bodies: gear_bodies,
            plan,
        }) = self.form_gears(source)?
        else {
            return Ok(None);
        };
        let (gear_bodies, merged, generated) = match plan {
            ModelPlan::Merged(mut m) => {
                let generated =
                    crate::form_mesh::point_at(generated, &m.mesh_path, &m.skeleton_path)?;
                (std::mem::take(&mut m.gear_bodies), Some(m), generated)
            }
            ModelPlan::Shared => (gear_bodies, None, generated.to_vec()),
            ModelPlan::Refused(reason) => {
                warn!(
                    alias = %self.alias,
                    reason = %reason,
                    "A form of this skin uses another mesh or skeleton that cannot share one model with the others; the skin keeps its first form"
                );
                return Ok(None);
            }
        };
        let generated = generated.as_slice();
        let swaps = swaps_of(&gear_bodies)?;
        let mut coverage = crate::form_gear::gear_coverage(source, &gear_bodies, &swaps)?;
        coverage.model = merged.as_ref().map(|m| m.outcome());
        if coverage.mesh_swap {
            warn!(
                alias = %self.alias,
                "A form of this skin uses another mesh or skeleton; Ctrl+5 cannot show it, so the skin keeps its first form"
            );
            return Ok(None);
        }
        let Some((mesh_path, skn)) = self.cycle_mesh(merged.as_ref(), generated)? else {
            return Ok(None);
        };
        let mesh_parts: BTreeSet<String> = crate::forms::skn_submesh_names(&skn)?
            .into_iter()
            .map(|name| name.to_ascii_lowercase())
            .collect();
        let initially_hidden: BTreeSet<u32> =
            crate::form_marker::mesh_text(generated, "initialSubmeshToHide")?
                .unwrap_or_default()
                .split(|c: char| c.is_whitespace() || matches!(c, ',' | '|' | ':'))
                .filter(|name| !name.is_empty())
                .map(prop_key_hash)
                .collect();
        let mut materials =
            crate::form_gear::material_plan(&gear_bodies, &swaps, &initially_hidden)?;
        materials
            .copies
            .retain(|copy| mesh_parts.contains(&copy.source.to_ascii_lowercase()));
        let swaps = crate::form_gear::with_copies(&swaps, &materials.copies);
        let copies: Vec<(String, String)> = materials
            .copies
            .iter()
            .map(|c| (c.source.clone(), c.name.clone()))
            .collect();
        let parsed = parse_prop_file(source).map_err(bin_error)?;
        let animations = self.animations(&parsed.links)?;
        let Some(super::form_steps::Markers {
            names,
            keys: markers,
        }) = self.cycle_markers(merged.as_ref(), swaps.len(), &animations)?
        else {
            return Ok(None);
        };
        let mesh = if names.is_empty() && copies.is_empty() {
            Some(skn)
        } else {
            crate::form_marker::add_parts(&skn, &names, &copies)
        };
        let Some(mesh) = mesh else {
            debug!(
                alias = %self.alias,
                mesh = %mesh_path,
                copies = ?copies,
                "Form cycle skipped: the mesh cannot take the markers and per-form part copies"
            );
            return Ok(None);
        };
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
        .map_err(bin_error)?
        .and_then(|v| v.as_u32());
        let Some(graph) = graph else {
            debug!(alias = %self.alias, "Form cycle skipped: the skin names no animation graph");
            return Ok(None);
        };
        if graph == prop_key_hash(&format!("Characters/{}/Animations/Skin0", self.alias)) {
            debug!(alias = %self.alias, "Form cycle skipped: the skin plays the default skin's graph");
            return Ok(None);
        }
        let Some(mut graph_file) = self.cycle_graph(
            source,
            graph,
            merged.as_ref(),
            &swaps,
            &markers,
            &animations,
        )?
        else {
            return Ok(None);
        };
        crate::form_trace::trace(&crate::form_trace::FormTrace {
            wad: &self.wad,
            alias: &self.alias,
            skin_bin: source,
            graph_path: &graph_file.0,
            graph_key: graph,
            generated_graph: &graph_file.1,
            swaps: &swaps,
            markers: &std::iter::once(0)
                .chain(markers.iter().copied())
                .collect::<Vec<_>>(),
        });
        let form_drivers = crate::form_state::form_drivers(&markers);
        let mut drivers = 0;
        let skin0 = match crate::gear_toggle::drive_by_parts(generated, &form_drivers)? {
            Some((bytes, count)) => {
                drivers += count;
                bytes
            }
            None => generated.to_vec(),
        };
        let hidden: Vec<String> = names
            .iter()
            .chain(copies.iter().map(|(_, name)| name))
            .chain(merged.iter().flat_map(|m| m.extra_parts()))
            .cloned()
            .collect();
        let skin0 = crate::form_marker::hide_at_start(&skin0, &hidden)?.unwrap_or(skin0);
        let skin0 = crate::form_state::persist_forms(&skin0, &swaps, &markers)?.unwrap_or(skin0);
        let skin0 = crate::form_gear::apply_gears(&skin0, &gear_bodies, &markers, &materials)?
            .unwrap_or(skin0);
        let mut effect_bins = vec![source.to_vec()];
        for link in parsed
            .links
            .iter()
            .filter(|l| !l.to_ascii_lowercase().contains("/animations/"))
        {
            effect_bins.extend(self.wad.read(wad_path_hash(&link.to_ascii_lowercase()))?);
        }
        let bone = match (&merged, crate::form_marker::mesh_text(&skin0, "skeleton")?) {
            (Some(m), _) => crate::vfx_markers::ground_bone(&m.skeleton),
            (None, Some(path)) => self
                .wad
                .read(wad_path_hash(&path.to_ascii_lowercase()))?
                .and_then(|skl| crate::vfx_markers::ground_bone(&skl)),
            (None, None) => None,
        };
        let (skin0, effects) = match &bone {
            Some(bone) => crate::form_vfx::gate_form_effects(
                &skin0,
                &crate::form_vfx::FormEffects {
                    bins: &effect_bins,
                    gear_bodies: &gear_bodies,
                    markers: &markers,
                    bone,
                },
            )?
            .unwrap_or((skin0, 0)),
            None => (skin0, 0),
        };
        let mut files = Vec::new();
        let skin0 = match self.form_audio(source, &graph_file.1, graph, swaps.len(), &skin0) {
            Some(audio) => {
                graph_file.1 = audio.graph;
                files.push((audio.bank_path.to_ascii_lowercase(), audio.bank));
                audio.skin0
            }
            None => skin0,
        };
        files.push(graph_file);
        files.push((mesh_path, mesh));
        if let Some(m) = merged {
            files.push((m.skeleton_path.to_ascii_lowercase(), m.skeleton));
        }
        let skin0 = crate::forms::strip_gear_indicators(&skin0)?;
        if let Some((bytes, count)) = crate::gear_toggle::drive_by_parts(source, &form_drivers)? {
            drivers += count;
            files.push((source_path.to_owned(), bytes));
        }
        Ok(Some(FormCycle {
            files,
            skin0,
            forms: swaps.len(),
            drivers,
            effects,
            coverage,
        }))
    }

    pub(crate) fn with_form_cycle(
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
                    effect_emitters = plan.effects,
                    idle_forms = plan.coverage.idle_forms,
                    material_parts = plan.coverage.material_parts,
                    part_copies = plan.coverage.part_copies,
                    model = ?plan.coverage.model,
                    files = ?plan.files.iter().map(|(path, _)| path.as_str()).collect::<Vec<_>>(),
                    "Ctrl+5 cycles the skin's forms in game"
                );
                let limits = &plan.coverage;
                if !limits.per_form_look.is_empty() || limits.script_states > 0 {
                    warn!(
                        alias = %self.alias,
                        skin,
                        per_form_look = ?limits.per_form_look,
                        script_states = limits.script_states,
                        "Some of this skin's form changes cannot follow Ctrl+5 and stay as the first form shows them"
                    );
                }
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
}
