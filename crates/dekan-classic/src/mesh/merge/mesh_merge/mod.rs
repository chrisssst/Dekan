use std::collections::{HashMap, HashSet};

use super::skeleton::{Joint, Skeleton, Transform, joint_hash};
use super::skinned_mesh::{
    BASIC_VERTEX, COLOR_AT, COLOR_VERTEX, INFLUENCES_AT, SkinnedMesh, Submesh, TANGENT_VERTEX,
};
use crate::error::ClassicError;

pub(crate) const MAX_INFLUENCES: usize = 256;
pub(crate) const MAX_JOINTS: usize = 255;
pub(crate) const MAX_VERTICES: usize = 65_535;
const COLOR_VERTEX_TYPE: u32 = 1;
const WEIGHTS_AT: usize = 16;
const WHITE: [u8; 4] = [u8::MAX; 4];
const TWIN_PREFIX: &str = "DekanForm";

pub struct FormModel {
    pub skeleton: Skeleton,
    pub mesh: SkinnedMesh,
}

#[derive(Debug)]
pub struct MergedForms {
    pub skeleton: Skeleton,
    pub mesh: SkinnedMesh,
    pub parts: Vec<Vec<String>>,
    pub twins: usize,
}

pub(crate) fn refuse(why: impl std::fmt::Display) -> ClassicError {
    ClassicError::Mesh(format!("forms cannot share one model: {why}"))
}

fn vertex_layout(forms: &[FormModel]) -> Result<(usize, u32), ClassicError> {
    let sizes: HashSet<usize> = forms.iter().map(|f| f.mesh.vertex_size).collect();
    if let Some(odd) = sizes
        .iter()
        .find(|s| ![BASIC_VERTEX, COLOR_VERTEX, TANGENT_VERTEX].contains(s))
    {
        return Err(refuse(format!("vertex size {odd} is not supported")));
    }
    match sizes.len() {
        1 => Ok((forms[0].mesh.vertex_size, forms[0].mesh.vertex_type)),
        2 if sizes.contains(&BASIC_VERTEX) && sizes.contains(&COLOR_VERTEX) => {
            Ok((COLOR_VERTEX, COLOR_VERTEX_TYPE))
        }
        _ => Err(refuse(format!(
            "vertex layouts {sizes:?} cannot be unified"
        ))),
    }
}

struct Joints {
    list: Vec<Joint>,
    by_name: HashMap<String, usize>,
    influences: Vec<u16>,
    twins: usize,
}

impl Joints {
    fn new(base: &Skeleton) -> Self {
        Self {
            by_name: base
                .joints
                .iter()
                .enumerate()
                .map(|(i, j)| (j.name.to_ascii_lowercase(), i))
                .collect(),
            list: base.joints.clone(),
            influences: base.influences.clone(),
            twins: 0,
        }
    }

    fn push(&mut self, mut joint: Joint) -> Result<usize, ClassicError> {
        let index = self.list.len();
        if index >= MAX_JOINTS {
            return Err(refuse(format!("more than {MAX_JOINTS} joints")));
        }
        joint.id = i16::try_from(index).map_err(|_| refuse("joint id overflow"))?;
        self.by_name.insert(joint.name.to_ascii_lowercase(), index);
        self.list.push(joint);
        Ok(index)
    }

    fn adopt(&mut self, skeleton: &Skeleton) -> Result<(), ClassicError> {
        for joint in &skeleton.joints {
            if self.by_name.contains_key(&joint.name.to_ascii_lowercase()) {
                continue;
            }
            let parent = match usize::try_from(joint.parent) {
                Ok(at) => {
                    let name = skeleton
                        .joints
                        .get(at)
                        .map(|p| p.name.to_ascii_lowercase())
                        .ok_or_else(|| refuse(format!("joint '{}' has no parent", joint.name)))?;
                    let index = *self.by_name.get(&name).ok_or_else(|| {
                        refuse(format!("joint '{}' comes before its parent", joint.name))
                    })?;
                    i16::try_from(index).map_err(|_| refuse("parent id overflow"))?
                }
                Err(_) => -1,
            };
            self.push(Joint {
                parent,
                ..joint.clone()
            })?;
        }
        Ok(())
    }

    fn skinning_joint(&mut self, form: usize, joint: &Joint) -> Result<usize, ClassicError> {
        let shared = self
            .by_name
            .get(&joint.name.to_ascii_lowercase())
            .copied()
            .ok_or_else(|| refuse(format!("joint '{}' was never adopted", joint.name)))?;
        if self.list[shared].inverse_bind.same_as(&joint.inverse_bind) {
            return Ok(shared);
        }
        let name = format!("{TWIN_PREFIX}{form}_{}", joint.name);
        if let Some(twin) = self.by_name.get(&name.to_ascii_lowercase()) {
            return Ok(*twin);
        }
        self.twins += 1;
        self.push(Joint {
            flags: joint.flags,
            id: 0,
            parent: i16::try_from(shared).map_err(|_| refuse("parent id overflow"))?,
            hash: joint_hash(&name),
            radius: joint.radius,
            local: Transform::IDENTITY,
            inverse_bind: joint.inverse_bind,
            name,
        })
    }

    fn influence_map(&mut self, form: usize, skeleton: &Skeleton) -> Result<Vec<u8>, ClassicError> {
        if form == 0 {
            return (0..skeleton.influences.len())
                .map(|i| u8::try_from(i).map_err(|_| refuse("too many influences")))
                .collect();
        }
        self.adopt(skeleton)?;
        let mut map = Vec::with_capacity(skeleton.influences.len());
        for influence in &skeleton.influences {
            let joint = skeleton
                .joints
                .get(usize::from(*influence))
                .ok_or_else(|| refuse(format!("influence {influence} names no joint")))?;
            let target = self.skinning_joint(form, joint)?;
            let position = match self
                .influences
                .iter()
                .position(|i| usize::from(*i) == target)
            {
                Some(at) => at,
                None => {
                    self.influences
                        .push(u16::try_from(target).map_err(|_| refuse("joint index overflow"))?);
                    self.influences.len() - 1
                }
            };
            if position >= MAX_INFLUENCES {
                return Err(refuse(format!("more than {MAX_INFLUENCES} influences")));
            }
            map.push(u8::try_from(position).map_err(|_| refuse("influence overflow"))?);
        }
        Ok(map)
    }
}

fn weight(vertex: &[u8], slot: usize) -> f32 {
    crate::binary::array(vertex, WEIGHTS_AT + slot * 4).map_or(0.0, f32::from_le_bytes)
}

fn append_vertices(
    target: &mut SkinnedMesh,
    source: &SkinnedMesh,
    map: &[u8],
) -> Result<(), ClassicError> {
    for index in 0..source.vertex_count() {
        let vertex = source
            .vertex(index)
            .ok_or_else(|| refuse(format!("vertex {index} is missing")))?;
        let mut out = vertex[..BASIC_VERTEX].to_vec();
        for slot in 0..4 {
            let at = INFLUENCES_AT + slot;
            out[at] = match map.get(usize::from(out[at])) {
                Some(mapped) => *mapped,
                None if weight(vertex, slot) == 0.0 => 0,
                None => {
                    return Err(refuse(format!(
                        "vertex {index} names influence {}",
                        out[at]
                    )));
                }
            };
        }
        match (target.vertex_size, vertex.len()) {
            (COLOR_VERTEX, BASIC_VERTEX) => out.extend_from_slice(&WHITE),
            (COLOR_VERTEX, COLOR_VERTEX) | (TANGENT_VERTEX, TANGENT_VERTEX) => {
                out.extend_from_slice(&vertex[COLOR_AT..]);
            }
            (BASIC_VERTEX, BASIC_VERTEX) => {}
            (to, from) => return Err(refuse(format!("vertex of {from} bytes cannot become {to}"))),
        }
        target.vertices.extend_from_slice(&out);
    }
    Ok(())
}

fn widen_bounds(target: &mut [f32; 10], other: &[f32; 10]) {
    for axis in 0..3 {
        target[axis] = target[axis].min(other[axis]);
        target[axis + 3] = target[axis + 3].max(other[axis + 3]);
    }
}

fn sphere(bounds: &mut [f32; 10], forms: &[FormModel]) {
    let center = [0, 1, 2].map(|axis| (bounds[axis] + bounds[axis + 3]) / 2.0);
    let radius = forms
        .iter()
        .map(|f| {
            let b = &f.mesh.bounds;
            let distance = (0..3)
                .map(|axis| (b[6 + axis] - center[axis]).powi(2))
                .sum::<f32>()
                .sqrt();
            distance + b[9]
        })
        .fold(0.0, f32::max);
    bounds[6..9].copy_from_slice(&center);
    bounds[9] = radius;
}

pub fn merge_forms(forms: &[FormModel]) -> Result<MergedForms, ClassicError> {
    let base = forms.first().ok_or_else(|| refuse("no forms"))?;
    let (vertex_size, vertex_type) = vertex_layout(forms)?;
    let total: usize = forms.iter().map(|f| f.mesh.vertex_count()).sum();
    if total > MAX_VERTICES {
        return Err(refuse(format!(
            "{total} vertices, more than {MAX_VERTICES}"
        )));
    }
    let mut joints = Joints::new(&base.skeleton);
    let mut mesh = SkinnedMesh {
        minor: base.mesh.minor,
        flags: base.mesh.flags,
        vertex_type,
        vertex_size,
        bounds: base.mesh.bounds,
        submeshes: Vec::new(),
        indices: Vec::new(),
        vertices: Vec::new(),
        trailer: base.mesh.trailer.clone(),
    };
    let mut seen = HashSet::new();
    let mut parts = Vec::with_capacity(forms.len());
    for (form, model) in forms.iter().enumerate() {
        let map = joints.influence_map(form, &model.skeleton)?;
        let first_vertex = mesh.vertex_count();
        let first_index = mesh.indices.len();
        append_vertices(&mut mesh, &model.mesh, &map)?;
        for index in &model.mesh.indices {
            let shifted = usize::from(*index) + first_vertex;
            mesh.indices
                .push(u16::try_from(shifted).map_err(|_| refuse("index overflow"))?);
        }
        let mut names = Vec::with_capacity(model.mesh.submeshes.len());
        for part in &model.mesh.submeshes {
            if !seen.insert(part.name.to_ascii_lowercase()) {
                return Err(refuse(format!(
                    "two forms have a part named '{}'",
                    part.name
                )));
            }
            let offset = |value: u32, by: usize| {
                u32::try_from(by)
                    .ok()
                    .and_then(|by| value.checked_add(by))
                    .ok_or_else(|| refuse("part offset overflow"))
            };
            mesh.submeshes.push(Submesh {
                name: part.name.clone(),
                first_vertex: offset(part.first_vertex, first_vertex)?,
                vertex_count: part.vertex_count,
                first_index: offset(part.first_index, first_index)?,
                index_count: part.index_count,
            });
            names.push(part.name.clone());
        }
        widen_bounds(&mut mesh.bounds, &model.mesh.bounds);
        parts.push(names);
    }
    sphere(&mut mesh.bounds, forms);
    let twins = joints.twins;
    Ok(MergedForms {
        skeleton: Skeleton {
            joints: joints.list,
            influences: joints.influences,
            ..base.skeleton.clone()
        },
        mesh,
        parts,
        twins,
    })
}

#[cfg(test)]
mod tests;
