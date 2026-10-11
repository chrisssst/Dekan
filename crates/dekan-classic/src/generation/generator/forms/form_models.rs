use std::collections::BTreeMap;
use std::collections::btree_map::Entry;

use super::*;
use crate::mesh_merge::{FormModel, refuse};

pub(crate) struct MergedModels {
    pub gear_bodies: Vec<Vec<u8>>,
    pub mesh_path: String,
    pub mesh: Vec<u8>,
    pub skeleton_path: String,
    pub skeleton: Vec<u8>,
    pub parts: Vec<Vec<String>>,
    pub part_sizes: BTreeMap<String, u32>,
    pub twins: usize,
    pub joints: usize,
    pub tokens: Vec<Option<String>>,
}

impl MergedModels {
    pub(crate) fn extra_parts(&self) -> impl Iterator<Item = &String> {
        self.parts.iter().skip(1).flatten()
    }

    pub(crate) fn part_size(&self, part: &str) -> u32 {
        self.part_sizes
            .get(&part.to_ascii_lowercase())
            .copied()
            .unwrap_or(0)
    }

    pub(crate) fn outcome(&self) -> crate::form_gear::ModelOutcome {
        crate::form_gear::ModelOutcome::Merged {
            forms: self.parts.len(),
            twins: self.twins,
        }
    }
}

pub(crate) struct FormGears {
    pub bodies: Vec<Vec<u8>>,
    pub plan: ModelPlan,
}

pub(crate) enum ModelPlan {
    Shared,
    Merged(MergedModels),
    Refused(String),
}

impl StandardChampion {
    pub(crate) fn form_gears(&self, source: &[u8]) -> Result<Option<FormGears>, ClassicError> {
        let keys = crate::forms::gear_keys(source)?;
        if keys.len() < 2 || self.gear_count(0) > 0 {
            return Ok(None);
        }
        let bodies = keys
            .iter()
            .map(|key| self.gear_body(source, *key))
            .collect::<Result<Vec<_>, _>>()?;
        if bodies.iter().all(|body| *body == bodies[0]) {
            return Ok(None);
        }
        let plan = match self.merged_models(source, &bodies) {
            Ok(Some(merged)) => ModelPlan::Merged(merged),
            Ok(None) => ModelPlan::Shared,
            Err(e) => ModelPlan::Refused(e.to_string()),
        };
        Ok(Some(FormGears { bodies, plan }))
    }

    fn merged_models(
        &self,
        source: &[u8],
        gear_bodies: &[Vec<u8>],
    ) -> Result<Option<MergedModels>, ClassicError> {
        let models = crate::form_mesh::gear_models(gear_bodies)?;
        let base_mesh = crate::form_marker::mesh_text(source, "simpleSkin")?;
        let base_skeleton = crate::form_marker::mesh_text(source, "skeleton")?;
        let differs = |own: Option<&str>, base: Option<&str>| {
            own.is_some_and(|path| base.is_none_or(|base| !base.eq_ignore_ascii_case(path)))
        };
        if !models.iter().any(|m| {
            differs(m.mesh.as_deref(), base_mesh.as_deref())
                || differs(m.skeleton.as_deref(), base_skeleton.as_deref())
        }) {
            return Ok(None);
        }
        let (Some(base_mesh), Some(base_skeleton)) = (base_mesh, base_skeleton) else {
            return Err(refuse("the skin names no mesh or skeleton of its own"));
        };
        if models.iter().any(|m| m.scale != models[0].scale) {
            return Err(refuse("the forms are drawn at different scales"));
        }
        let mut skeletons = BTreeMap::new();
        let mut meshes = BTreeMap::new();
        let mut forms = Vec::with_capacity(models.len());
        for model in &models {
            let skeleton = model
                .skeleton
                .as_deref()
                .unwrap_or(&base_skeleton)
                .to_ascii_lowercase();
            let mesh = model
                .mesh
                .as_deref()
                .unwrap_or(&base_mesh)
                .to_ascii_lowercase();
            let skeleton = match skeletons.entry(skeleton) {
                Entry::Occupied(known) => known.into_mut(),
                Entry::Vacant(new) => {
                    let parsed = crate::skeleton::parse(&self.model_file(new.key())?)?;
                    new.insert(parsed)
                }
            }
            .clone();
            let mesh = match meshes.entry(mesh) {
                Entry::Occupied(known) => known.into_mut(),
                Entry::Vacant(new) => {
                    let parsed = crate::skinned_mesh::parse(&self.model_file(new.key())?)?;
                    new.insert(parsed)
                }
            }
            .clone();
            forms.push(FormModel { skeleton, mesh });
        }
        let merged = crate::mesh_merge::merge_forms(&forms)?;
        let tokens = models
            .iter()
            .map(|m| {
                m.mesh
                    .as_deref()
                    .or(m.skeleton.as_deref())
                    .and_then(|path| crate::form_transition::form_token(&self.alias, path))
            })
            .collect();
        Ok(Some(MergedModels {
            gear_bodies: crate::form_mesh::as_part_gears(gear_bodies, &merged.parts)?,
            mesh_path: crate::form_mesh::merged_path(&base_mesh),
            mesh: crate::skinned_mesh::write(&merged.mesh)?,
            skeleton_path: crate::form_mesh::merged_path(&base_skeleton),
            skeleton: crate::skeleton::write(&merged.skeleton)?,
            joints: merged.skeleton.joints.len(),
            part_sizes: merged
                .mesh
                .submeshes
                .iter()
                .map(|s| (s.name.to_ascii_lowercase(), s.index_count))
                .collect(),
            tokens,
            parts: merged.parts,
            twins: merged.twins,
        }))
    }

    fn model_file(&self, path: &str) -> Result<Vec<u8>, ClassicError> {
        self.wad
            .read(wad_path_hash(path))?
            .ok_or_else(|| refuse(format!("'{path}' is not in the game")))
    }
}
