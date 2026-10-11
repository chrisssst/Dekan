use crate::binary::Reader;
use crate::error::ClassicError;

const TOKEN: u32 = 0x22FD_4FC3;
const HEADER: usize = 64;
const JOINT: usize = 100;
const INDEX_ENTRY: usize = 8;
const NAME_FIELD: usize = 96;
const RESERVED: usize = 5;
const POSITION_TOLERANCE: f32 = 0.01;
const SCALE_TOLERANCE: f32 = 0.001;
const ROTATION_DOT: f32 = 0.999_99;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    pub translation: [f32; 3],
    pub scale: [f32; 3],
    pub rotation: [f32; 4],
}

impl Transform {
    pub const IDENTITY: Self = Self {
        translation: [0.0; 3],
        scale: [1.0; 3],
        rotation: [0.0, 0.0, 0.0, 1.0],
    };

    #[must_use]
    pub fn same_as(&self, other: &Self) -> bool {
        let close = |a: &[f32], b: &[f32], tolerance: f32| {
            a.iter().zip(b).all(|(x, y)| (x - y).abs() <= tolerance)
        };
        let dot: f32 = self
            .rotation
            .iter()
            .zip(&other.rotation)
            .map(|(a, b)| a * b)
            .sum();
        close(&self.translation, &other.translation, POSITION_TOLERANCE)
            && close(&self.scale, &other.scale, SCALE_TOLERANCE)
            && dot.abs() >= ROTATION_DOT
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Joint {
    pub flags: u16,
    pub id: i16,
    pub parent: i16,
    pub hash: u32,
    pub radius: f32,
    pub local: Transform,
    pub inverse_bind: Transform,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Skeleton {
    pub version: u32,
    pub flags: u16,
    pub name: String,
    pub asset_name: String,
    pub joints: Vec<Joint>,
    pub influences: Vec<u16>,
}

#[must_use]
pub fn joint_hash(name: &str) -> u32 {
    name.bytes().fold(0u32, |hash, byte| {
        let hash = (hash << 4).wrapping_add(u32::from(byte.to_ascii_lowercase()));
        let high = hash & 0xF000_0000;
        let hash = if high == 0 { hash } else { hash ^ (high >> 24) };
        hash & !high
    })
}

fn corrupt(what: String) -> ClassicError {
    ClassicError::Mesh(format!("skeleton: {what}"))
}

fn floats<const N: usize>(r: &Reader, at: usize) -> Result<[f32; N], ClassicError> {
    let mut out = [0.0; N];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = r.f32(at + i * 4)?;
    }
    Ok(out)
}

fn transform(r: &Reader, at: usize) -> Result<Transform, ClassicError> {
    Ok(Transform {
        translation: floats(r, at)?,
        scale: floats(r, at + 12)?,
        rotation: floats(r, at + 24)?,
    })
}

fn offset(r: &Reader, at: usize) -> Result<usize, ClassicError> {
    let raw = r.i32(at)?;
    usize::try_from(raw).map_err(|_| r.fail(format!("negative offset {raw} at {at}")))
}

fn text(r: &Reader, at: usize) -> Result<String, ClassicError> {
    let rest = r
        .bytes
        .get(at..)
        .ok_or_else(|| r.fail(format!("string offset {at} past the end")))?;
    let end = rest
        .iter()
        .position(|b| *b == 0)
        .ok_or_else(|| r.fail(format!("unterminated string at {at}")))?;
    String::from_utf8(rest[..end].to_vec())
        .map_err(|_| r.fail(format!("string at {at} is not UTF-8")))
}

pub fn parse(bytes: &[u8]) -> Result<Skeleton, ClassicError> {
    let r = Reader::new(bytes, corrupt);
    if r.u32(4)? != TOKEN {
        return Err(r.fail("not a skeleton in the current format"));
    }
    let joint_count = r.u16(14)?;
    let influence_count = r.u32(16)? as usize;
    let joints_at = offset(&r, 20)?;
    let influences_at = offset(&r, 28)?;
    let mut joints = Vec::with_capacity(usize::from(joint_count).min(bytes.len() / JOINT));
    for i in 0..usize::from(joint_count) {
        let at = joints_at + i * JOINT;
        let relative = r.i32(at + NAME_FIELD)?;
        let name_at = (at + NAME_FIELD)
            .checked_add_signed(relative as isize)
            .ok_or_else(|| r.fail(format!("joint {i} name offset points outside the file")))?;
        let parent = r.i16(at + 4)?;
        if parent < -1 || i32::from(parent) >= i32::from(joint_count) {
            return Err(r.fail(format!("joint {i} has parent {parent} of {joint_count}")));
        }
        joints.push(Joint {
            flags: r.u16(at)?,
            id: r.i16(at + 2)?,
            parent,
            hash: r.u32(at + 8)?,
            radius: r.f32(at + 12)?,
            local: transform(&r, at + 16)?,
            inverse_bind: transform(&r, at + 56)?,
            name: text(&r, name_at)?,
        });
    }
    let influences = (0..influence_count)
        .map(|i| r.u16(influences_at + i * 2))
        .collect::<Result<Vec<_>, _>>()?;
    if let Some(bad) = influences.iter().find(|i| **i >= joint_count) {
        return Err(r.fail(format!("influence names joint {bad} of {joint_count}")));
    }
    Ok(Skeleton {
        version: r.u32(8)?,
        flags: r.u16(12)?,
        name: text(&r, offset(&r, 32)?)?,
        asset_name: text(&r, offset(&r, 36)?)?,
        joints,
        influences,
    })
}

fn pad4(out: &mut Vec<u8>) {
    while out.len() % 4 != 0 {
        out.push(0);
    }
}

fn put_text(out: &mut Vec<u8>, text: &str) {
    out.extend_from_slice(text.as_bytes());
    out.push(0);
    pad4(out);
}

fn put_transform(out: &mut Vec<u8>, t: &Transform) {
    for value in t.translation.iter().chain(&t.scale).chain(&t.rotation) {
        out.extend_from_slice(&value.to_le_bytes());
    }
}

fn as_i32(value: usize) -> Result<i32, ClassicError> {
    i32::try_from(value).map_err(|_| corrupt(format!("offset {value} does not fit")))
}

pub fn write(skeleton: &Skeleton) -> Result<Vec<u8>, ClassicError> {
    let count = u16::try_from(skeleton.joints.len())
        .map_err(|_| corrupt(format!("{} joints do not fit", skeleton.joints.len())))?;
    let joints_at = HEADER;
    let indices_at = joints_at + skeleton.joints.len() * JOINT;
    let influences_at = indices_at + skeleton.joints.len() * INDEX_ENTRY;
    let mut tail = Vec::new();
    for influence in &skeleton.influences {
        tail.extend_from_slice(&influence.to_le_bytes());
    }
    pad4(&mut tail);
    let name_at = influences_at + tail.len();
    put_text(&mut tail, &skeleton.name);
    let asset_at = if !skeleton.name.is_empty() && skeleton.asset_name == skeleton.name {
        name_at
    } else {
        let at = influences_at + tail.len();
        put_text(&mut tail, &skeleton.asset_name);
        at
    };
    let names_at = influences_at + tail.len();
    let mut body = Vec::with_capacity(skeleton.joints.len() * (JOINT + INDEX_ENTRY));
    for (i, joint) in skeleton.joints.iter().enumerate() {
        let relative =
            as_i32(influences_at + tail.len())? - as_i32(joints_at + i * JOINT + NAME_FIELD)?;
        put_text(&mut tail, &joint.name);
        body.extend_from_slice(&joint.flags.to_le_bytes());
        body.extend_from_slice(&joint.id.to_le_bytes());
        body.extend_from_slice(&joint.parent.to_le_bytes());
        body.extend_from_slice(&0u16.to_le_bytes());
        body.extend_from_slice(&joint.hash.to_le_bytes());
        body.extend_from_slice(&joint.radius.to_le_bytes());
        put_transform(&mut body, &joint.local);
        put_transform(&mut body, &joint.inverse_bind);
        body.extend_from_slice(&relative.to_le_bytes());
    }
    let mut index: Vec<(u32, u16)> = skeleton
        .joints
        .iter()
        .map(|j| j.hash)
        .zip(0..count)
        .collect();
    index.sort_by_key(|(hash, _)| *hash);
    for (hash, id) in &index {
        body.extend_from_slice(&id.to_le_bytes());
        body.extend_from_slice(&0u16.to_le_bytes());
        body.extend_from_slice(&hash.to_le_bytes());
    }
    let total = influences_at + tail.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(&as_i32(total)?.to_le_bytes());
    out.extend_from_slice(&TOKEN.to_le_bytes());
    out.extend_from_slice(&skeleton.version.to_le_bytes());
    out.extend_from_slice(&skeleton.flags.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    let influences = u32::try_from(skeleton.influences.len())
        .map_err(|_| corrupt("too many influences".into()))?;
    out.extend_from_slice(&influences.to_le_bytes());
    for at in [
        joints_at,
        indices_at,
        influences_at,
        name_at,
        asset_at,
        names_at,
    ] {
        out.extend_from_slice(&as_i32(at)?.to_le_bytes());
    }
    (0..RESERVED).for_each(|_| out.extend_from_slice(&(-1i32).to_le_bytes()));
    out.extend_from_slice(&body);
    out.extend_from_slice(&tail);
    Ok(out)
}

#[cfg(test)]
pub(crate) mod tests;
