use tracing::warn;

use crate::binary::u32_le;

const SKL_FORMAT_TOKEN: u32 = 0x22fd_4fc3;
const SKL_LEGACY_MAGIC: &[u8; 8] = b"r3d2sklt";
const SKN_MAGIC: u32 = 0x0011_2233;
const MESH_PADDING: [u8; 16] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];

const NULL_JOINT_NAME: &[u8] = b"null_bone";
const SKL_HEADER_SIZE: usize = 64;
const SKL_VERSION: usize = 8;
const SKL_JOINT_COUNT: usize = 14;
const SKL_INFLUENCE_COUNT: usize = 16;
const SKL_SECTION_OFFSETS: usize = 20;
const SKL_JOINT_SIZE: usize = 100;
const JOINT_NAME_OFFSET: usize = 96;
const SKL_TABLE_ENTRY_SIZE: usize = 8;
const JOINT_RADIUS: f32 = 2.1;

const SKN_RANGE_SIZE: usize = 80;
const RANGE_VERTEX_START: usize = 64;
const RANGE_VERTEX_COUNT: usize = 68;
const LEGACY_VERTEX_SIZE: usize = 52;
const SKN_BOUNDS_SIZE: usize = 40;

fn u16_le(data: &[u8], at: usize) -> Option<u16> {
    let slice: [u8; 2] = data.get(at..at + 2)?.try_into().ok()?;
    Some(u16::from_le_bytes(slice))
}

fn i32_le(data: &[u8], at: usize) -> Option<i32> {
    let slice: [u8; 4] = data.get(at..at + 4)?.try_into().ok()?;
    Some(i32::from_le_bytes(slice))
}

fn elf_hash_lower(name: &[u8]) -> u32 {
    let mut value = 0u32;
    for &raw in name {
        let byte = if raw.is_ascii_uppercase() {
            raw + 32
        } else {
            raw
        };
        value = value.wrapping_shl(4).wrapping_add(u32::from(byte));
        let high = value & 0xf000_0000;
        if high != 0 {
            value ^= high >> 24;
        }
        value &= !high;
    }
    value
}

fn section_range(data: &[u8], offset: i32, length: usize) -> Option<(usize, usize)> {
    if offset <= 0 {
        return None;
    }
    let start = usize::try_from(offset).ok()?;
    let end = start.checked_add(length)?;
    (end <= data.len()).then_some((start, end))
}

fn cstr_range(data: &[u8], offset: i32) -> Option<(usize, usize)> {
    let start = usize::try_from(offset).ok()?;
    let rel_end = data.get(start..)?.iter().position(|&b| b == 0)?;
    Some((start, start + rel_end + 1))
}

fn build_null_joint(joint_id: u16, name_hash: u32, name_offset: i32) -> [u8; SKL_JOINT_SIZE] {
    let mut buf = [0u8; SKL_JOINT_SIZE];
    buf[0..2].copy_from_slice(&0u16.to_le_bytes());
    buf[2..4].copy_from_slice(&(joint_id as i16).to_le_bytes());
    buf[4..6].copy_from_slice(&(-1i16).to_le_bytes());
    buf[6..8].copy_from_slice(&0i16.to_le_bytes());
    buf[8..12].copy_from_slice(&name_hash.to_le_bytes());
    buf[12..16].copy_from_slice(&JOINT_RADIUS.to_le_bytes());
    let identity: [f32; 10] = [0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0];
    for copy in 0..2 {
        for (i, &v) in identity.iter().enumerate() {
            let at = 16 + copy * 40 + i * 4;
            buf[at..at + 4].copy_from_slice(&v.to_le_bytes());
        }
    }
    buf[96..100].copy_from_slice(&name_offset.to_le_bytes());
    buf
}

fn add_null_joint(data: &[u8]) -> Result<Vec<u8>, &'static str> {
    if data.len() < SKL_HEADER_SIZE {
        return Err("skeleton header truncated");
    }
    let version = u32_le(data, SKL_VERSION).ok_or("skeleton version missing")?;
    if version != 0 {
        return Err("skeleton version not supported");
    }
    let count = usize::from(u16_le(data, SKL_JOINT_COUNT).ok_or("joint count missing")?);
    if count == 0 || count >= 0x7fff {
        return Err("invalid skeleton joint count");
    }
    let mut offsets = [0i32; 6];
    for (i, slot) in offsets.iter_mut().enumerate() {
        *slot = i32_le(data, SKL_SECTION_OFFSETS + i * 4).ok_or("section offset missing")?;
    }
    let joints =
        section_range(data, offsets[0], count * SKL_JOINT_SIZE).ok_or("joints out of bounds")?;
    let table = section_range(data, offsets[1], count * SKL_TABLE_ENTRY_SIZE)
        .ok_or("joint table out of bounds")?;
    if joints.0 < table.1 && table.0 < joints.1 {
        return Err("joints and joint table overlap");
    }

    let mut strings = Vec::with_capacity(count + 2);
    for i in 0..count {
        let field = joints.0 + i * SKL_JOINT_SIZE + JOINT_NAME_OFFSET;
        let rel = i32_le(data, field).ok_or("joint name offset missing")?;
        let target = i32::try_from(field)
            .ok()
            .and_then(|f| f.checked_add(rel))
            .ok_or("joint name offset overflow")?;
        strings.push(cstr_range(data, target).ok_or("joint name out of bounds")?);
    }
    for &idx in &[3usize, 4usize] {
        if offsets[idx] > 0 {
            strings.push(cstr_range(data, offsets[idx]).ok_or("skeleton name out of bounds")?);
        }
    }
    let mut sections = vec![joints, table];
    if offsets[2] > 0 {
        let infl_count =
            usize::try_from(u32_le(data, SKL_INFLUENCE_COUNT).ok_or("influence count missing")?)
                .map_err(|_| "influence count overflow")?;
        let infl =
            section_range(data, offsets[2], infl_count * 2).ok_or("influences out of bounds")?;
        sections.push(infl);
    }
    sections.extend_from_slice(&strings);

    let null_hash = elf_hash_lower(NULL_JOINT_NAME);
    let mut entries: Vec<[u8; SKL_TABLE_ENTRY_SIZE]> = Vec::with_capacity(count + 1);
    for i in 0..count {
        let at = table.0 + i * SKL_TABLE_ENTRY_SIZE;
        let mut entry = [0u8; SKL_TABLE_ENTRY_SIZE];
        entry.copy_from_slice(&data[at..at + SKL_TABLE_ENTRY_SIZE]);
        if u32::from_le_bytes([entry[4], entry[5], entry[6], entry[7]]) == null_hash {
            return Ok(data.to_vec());
        }
        entries.push(entry);
    }

    let joint_insert = joints.1;
    let table_insert = table.1;
    for &(start, end) in &sections {
        if (start < joint_insert && joint_insert < end)
            || (start < table_insert && table_insert < end)
        {
            return Err("skeleton sections overlap");
        }
    }

    let content_end = sections.iter().map(|&(_, end)| end).max().unwrap_or(0);
    let tail = &data[content_end..];
    let body_end = if tail.len() < 4 && tail.iter().all(|&b| b == 0) {
        content_end
    } else {
        data.len()
    };

    let moved = |at: usize| -> usize {
        at + usize::from(at >= joint_insert) * SKL_JOINT_SIZE
            + usize::from(at >= table_insert) * SKL_TABLE_ENTRY_SIZE
    };

    let mut inserts = [
        (joint_insert, SKL_JOINT_SIZE),
        (table_insert, SKL_TABLE_ENTRY_SIZE),
    ];
    inserts.sort_by_key(|&(at, _)| at);

    let mut out = Vec::with_capacity(body_end + SKL_JOINT_SIZE + SKL_TABLE_ENTRY_SIZE + 16);
    let mut copied = 0usize;
    for (at, len) in inserts {
        out.extend_from_slice(&data[copied..at]);
        out.resize(out.len() + len, 0);
        copied = at;
    }
    out.extend_from_slice(&data[copied..body_end]);
    let name_at = out.len();
    out.extend_from_slice(NULL_JOINT_NAME);
    out.push(0);
    while out.len() % 4 != 0 {
        out.push(0);
    }

    let total_size = u32::try_from(out.len()).map_err(|_| "skeleton too large")?;
    out[0..4].copy_from_slice(&total_size.to_le_bytes());
    out[SKL_JOINT_COUNT..SKL_JOINT_COUNT + 2].copy_from_slice(&((count + 1) as u16).to_le_bytes());

    for (i, &off) in offsets.iter().enumerate() {
        if off > 0 {
            let new_off = i32::try_from(moved(off as usize)).map_err(|_| "offset overflow")?;
            let at = SKL_SECTION_OFFSETS + i * 4;
            out[at..at + 4].copy_from_slice(&new_off.to_le_bytes());
        }
    }
    for (i, &(name_start, _)) in strings.iter().take(count).enumerate() {
        let field = moved(joints.0 + i * SKL_JOINT_SIZE + JOINT_NAME_OFFSET);
        let rel = i32::try_from(moved(name_start) as isize - field as isize)
            .map_err(|_| "joint name relative offset overflow")?;
        out[field..field + 4].copy_from_slice(&rel.to_le_bytes());
    }

    let joint_at = moved(joint_insert) - SKL_JOINT_SIZE;
    let null_rel = i32::try_from(name_at as isize - (joint_at + JOINT_NAME_OFFSET) as isize)
        .map_err(|_| "null joint name offset overflow")?;
    let null_joint = build_null_joint(count as u16, null_hash, null_rel);
    out[joint_at..joint_at + SKL_JOINT_SIZE].copy_from_slice(&null_joint);

    let mut new_entry = [0u8; SKL_TABLE_ENTRY_SIZE];
    new_entry[0..2].copy_from_slice(&(count as i16).to_le_bytes());
    new_entry[4..8].copy_from_slice(&null_hash.to_le_bytes());
    let pos = entries
        .iter()
        .position(|other| u32::from_le_bytes([other[4], other[5], other[6], other[7]]) > null_hash)
        .unwrap_or(entries.len());
    entries.insert(pos, new_entry);
    let table_start = moved(table.0);
    for (i, entry) in entries.iter().enumerate() {
        let at = table_start + i * SKL_TABLE_ENTRY_SIZE;
        out[at..at + SKL_TABLE_ENTRY_SIZE].copy_from_slice(entry);
    }

    Ok(out)
}

fn add_null_vertex(data: &[u8]) -> Result<Vec<u8>, &'static str> {
    let major = u16_le(data, 4).ok_or("mesh header truncated")?;
    let mut at = 8usize;
    let mut last_range = None;
    match major {
        1 | 2 | 4 => {
            let range_count =
                usize::try_from(u32_le(data, at).ok_or("mesh range count truncated")?)
                    .map_err(|_| "range count overflow")?;
            at += 4;
            if range_count > 0 {
                last_range = Some(at + (range_count - 1) * SKN_RANGE_SIZE);
            }
            at = at
                .checked_add(
                    range_count
                        .checked_mul(SKN_RANGE_SIZE)
                        .ok_or("range overflow")?,
                )
                .ok_or("range offset overflow")?;
            if major == 4 {
                at += 4;
            }
        }
        0 => {}
        _ => return Err("mesh version not supported"),
    }

    let index_count = usize::try_from(u32_le(data, at).ok_or("index count truncated")?)
        .map_err(|_| "index count overflow")?;
    let vertex_count = usize::try_from(u32_le(data, at + 4).ok_or("vertex count truncated")?)
        .map_err(|_| "vertex count overflow")?;
    let vertex_count_at = at + 4;
    at += 8;

    let vertex_size = if major == 4 {
        let sz = usize::try_from(u32_le(data, at).ok_or("vertex size truncated")?)
            .map_err(|_| "vertex size overflow")?;
        at += 8 + SKN_BOUNDS_SIZE;
        sz
    } else {
        LEGACY_VERTEX_SIZE
    };
    if vertex_count == 0 || vertex_size == 0 {
        return Err("mesh has no vertices");
    }

    let indices_bytes = index_count.checked_mul(2).ok_or("index bytes overflow")?;
    let vertices_bytes = vertex_count
        .checked_mul(vertex_size)
        .ok_or("vertex bytes overflow")?;
    let vertices_end = at
        .checked_add(indices_bytes)
        .and_then(|v| v.checked_add(vertices_bytes))
        .ok_or("vertices end overflow")?;
    if vertices_end > data.len() {
        return Err("mesh vertices truncated");
    }

    let mut out = Vec::with_capacity(data.len() + vertex_size);
    out.extend_from_slice(&data[..vertices_end]);
    out.extend_from_slice(&data[vertices_end - vertex_size..vertices_end]);
    out.extend_from_slice(&data[vertices_end..]);

    let new_vcount = u32::try_from(vertex_count + 1).map_err(|_| "vertex count overflow")?;
    out[vertex_count_at..vertex_count_at + 4].copy_from_slice(&new_vcount.to_le_bytes());

    if let Some(range_at) = last_range {
        let start = usize::try_from(
            u32_le(data, range_at + RANGE_VERTEX_START).ok_or("range start truncated")?,
        )
        .map_err(|_| "range start overflow")?;
        let count = usize::try_from(
            u32_le(data, range_at + RANGE_VERTEX_COUNT).ok_or("range count truncated")?,
        )
        .map_err(|_| "range count overflow")?;
        if start.saturating_add(count) == vertex_count {
            let new_rcount = u32::try_from(count + 1).map_err(|_| "range vertex count overflow")?;
            let dst = range_at + RANGE_VERTEX_COUNT;
            out[dst..dst + 4].copy_from_slice(&new_rcount.to_le_bytes());
        }
    }

    Ok(out)
}

pub(super) fn rebuild_mesh(mut data: Vec<u8>) -> (Vec<u8>, &'static str) {
    if data.len() >= SKL_HEADER_SIZE && data[4..8] == SKL_FORMAT_TOKEN.to_le_bytes() {
        match add_null_joint(&data) {
            Ok(rebuilt) => return (rebuilt, "skl_null_joint"),
            Err(reason) => {
                warn!(
                    reason,
                    "Modern SKL structural rebuild failed; falling back to padding"
                );
                data.extend_from_slice(&MESH_PADDING);
                return (data, "skl_fallback_pad");
            }
        }
    }
    if data.len() >= 16 && &data[0..8] == SKL_LEGACY_MAGIC {
        let id = u32::from_le_bytes([data[12], data[13], data[14], data[15]]) ^ 0x424c_5431;
        data[12..16].copy_from_slice(&id.to_le_bytes());
        data.extend_from_slice(&MESH_PADDING);
        return (data, "skl_legacy_id");
    }
    if data.len() >= 12 && data[0..4] == SKN_MAGIC.to_le_bytes() {
        match add_null_vertex(&data) {
            Ok(rebuilt) => return (rebuilt, "skn_null_vertex"),
            Err(reason) => {
                warn!(
                    reason,
                    "SKN structural rebuild failed; falling back to padding"
                );
                data.extend_from_slice(&MESH_PADDING);
                return (data, "skn_fallback_pad");
            }
        }
    }
    data.extend_from_slice(&MESH_PADDING);
    (data, "fallback_pad")
}

#[cfg(test)]
mod tests;
