use dekan_wad::prop::tree::{self, Field, Value};
use dekan_wad::prop::{parse_prop_file, serialize_prop_file};

use crate::error::ClassicError;
use crate::gear_toggle::{bin_error, h};

const FIELD_STRING: u8 = 16;
const SKIN_CLASS: &str = "SkinCharacterDataProperties";
const SKN_MAGIC: u32 = 0x0011_2233;
const SKN_MAJOR: u16 = 4;
const PART_NAME: usize = 64;
const PART_SIZE: usize = PART_NAME + 16;
const PARTS_AT: usize = 12;
const BOUNDS_SIZE: usize = 40;
const MAX_PARTS: usize = 31;
const MAX_VERTICES: usize = 65_535;
const MARKER_VERTICES: usize = 3;

#[must_use]
pub fn marker_names(forms: usize) -> Vec<String> {
    (1..forms).map(|form| format!("DekanForm{form}")).collect()
}

fn count(bytes: &[u8], at: usize) -> Option<usize> {
    let raw = bytes.get(at..at.checked_add(4)?)?.try_into().ok()?;
    usize::try_from(u32::from_le_bytes(raw)).ok()
}

fn le32(value: usize) -> Option<[u8; 4]> {
    u32::try_from(value).ok().map(u32::to_le_bytes)
}

fn marker_vertex(size: usize) -> Vec<u8> {
    let mut vertex = vec![0; size];
    vertex[16..20].copy_from_slice(&1f32.to_le_bytes());
    vertex[36..40].copy_from_slice(&1f32.to_le_bytes());
    if size >= 56 {
        vertex[52..56].fill(u8::MAX);
    }
    vertex
}

fn part_name(raw: &[u8]) -> String {
    let end = raw.iter().position(|b| *b == 0).unwrap_or(raw.len());
    String::from_utf8_lossy(&raw[..end]).to_ascii_lowercase()
}

fn name_bytes(name: &str) -> Option<[u8; PART_NAME]> {
    if name.is_empty() || name.len() >= PART_NAME {
        return None;
    }
    let mut raw = [0u8; PART_NAME];
    raw[..name.len()].copy_from_slice(name.as_bytes());
    Some(raw)
}

#[must_use]
pub fn add_parts(skn: &[u8], markers: &[String], copies: &[(String, String)]) -> Option<Vec<u8>> {
    if (markers.is_empty() && copies.is_empty())
        || count(skn, 0)? != SKN_MAGIC as usize
        || skn.get(4..6)? != SKN_MAJOR.to_le_bytes()
    {
        return None;
    }
    let parts = count(skn, 8)?;
    let table_end = parts.checked_mul(PART_SIZE)?.checked_add(PARTS_AT)?;
    let index_count = count(skn, table_end.checked_add(4)?)?;
    let vertex_count = count(skn, table_end.checked_add(8)?)?;
    let vertex_size = count(skn, table_end.checked_add(12)?)?;
    let data = table_end.checked_add(20 + BOUNDS_SIZE)?;
    let indices_end = index_count.checked_mul(2)?.checked_add(data)?;
    let vertices_end = vertex_count
        .checked_mul(vertex_size)?
        .checked_add(indices_end)?;
    let added = markers.len() * MARKER_VERTICES;
    let new_parts = parts + copies.len() + markers.len();
    if new_parts > MAX_PARTS
        || vertex_count + added > MAX_VERTICES
        || !(52..=72).contains(&vertex_size)
        || vertices_end > skn.len()
    {
        return None;
    }

    let mut copied_indices = Vec::new();
    let mut copied_parts = Vec::new();
    for (source, name) in copies {
        let raw = name_bytes(name)?;
        let at = (0..parts)
            .map(|i| PARTS_AT + i * PART_SIZE)
            .find(|at| part_name(&skn[*at..*at + PART_NAME]) == source.to_ascii_lowercase())?;
        let range = |offset: usize| count(skn, at + PART_NAME + offset);
        let (first_vertex, vertices, first_index, indices) =
            (range(0)?, range(4)?, range(8)?, range(12)?);
        if first_index.checked_add(indices)? > index_count {
            return None;
        }
        let start = data + first_index * 2;
        let new_first = index_count + copied_indices.len() / 2;
        copied_indices.extend_from_slice(&skn[start..start + indices * 2]);
        copied_parts.push((raw, [first_vertex, vertices, new_first, indices]));
    }
    let copied = copied_indices.len() / 2;
    for (i, name) in markers.iter().enumerate() {
        let offset = i * MARKER_VERTICES;
        copied_parts.push((
            name_bytes(name)?,
            [
                vertex_count + offset,
                MARKER_VERTICES,
                index_count + copied + offset,
                MARKER_VERTICES,
            ],
        ));
    }

    let mut out = Vec::with_capacity(
        skn.len()
            + (copies.len() + markers.len()) * PART_SIZE
            + copied_indices.len()
            + added * (vertex_size + 2),
    );
    out.extend_from_slice(&skn[..8]);
    out.extend_from_slice(&le32(new_parts)?);
    out.extend_from_slice(&skn[PARTS_AT..table_end]);
    for (raw, values) in &copied_parts {
        out.extend_from_slice(raw);
        for value in values {
            out.extend_from_slice(&le32(*value)?);
        }
    }
    out.extend_from_slice(&skn[table_end..table_end + 4]);
    out.extend_from_slice(&le32(index_count + copied + added)?);
    out.extend_from_slice(&le32(vertex_count + added)?);
    out.extend_from_slice(&skn[table_end + 12..indices_end]);
    out.extend_from_slice(&copied_indices);
    for vertex in vertex_count..vertex_count + added {
        out.extend_from_slice(&u16::try_from(vertex).ok()?.to_le_bytes());
    }
    out.extend_from_slice(&skn[indices_end..vertices_end]);
    let vertex = marker_vertex(vertex_size);
    (0..added).for_each(|_| out.extend_from_slice(&vertex));
    out.extend_from_slice(&skn[vertices_end..]);
    Some(out)
}

fn mesh_fields(fields: &mut [Field]) -> Option<&mut Vec<Field>> {
    tree::field_mut(fields, h("skinMeshProperties")).and_then(Value::fields_mut)
}

fn text(value: &Value) -> Option<String> {
    match value {
        Value::Raw { kind, bytes } if *kind == FIELD_STRING && bytes.len() >= 2 => {
            Some(String::from_utf8_lossy(&bytes[2..]).into_owned())
        }
        _ => None,
    }
}

fn string_value(text: &str) -> Option<Value> {
    let len = u16::try_from(text.len()).ok()?;
    let mut bytes = len.to_le_bytes().to_vec();
    bytes.extend_from_slice(text.as_bytes());
    Some(Value::Raw {
        kind: FIELD_STRING,
        bytes,
    })
}

pub fn mesh_text(skin_bin: &[u8], field: &str) -> Result<Option<String>, ClassicError> {
    let file = parse_prop_file(skin_bin).map_err(bin_error)?;
    let Some(skin) = file.entries.iter().find(|e| e.class_hash == h(SKIN_CLASS)) else {
        return Ok(None);
    };
    let mut fields = tree::parse_fields(&skin.body).map_err(bin_error)?;
    Ok(mesh_fields(&mut fields)
        .and_then(|mesh| tree::field(mesh, h(field)))
        .and_then(text))
}

pub fn hide_at_start(skin_bin: &[u8], names: &[String]) -> Result<Option<Vec<u8>>, ClassicError> {
    let mut file = parse_prop_file(skin_bin).map_err(bin_error)?;
    let Some(at) = file
        .entries
        .iter()
        .position(|e| e.class_hash == h(SKIN_CLASS))
    else {
        return Ok(None);
    };
    let mut fields = tree::parse_fields(&file.entries[at].body).map_err(bin_error)?;
    let Some(mesh) = mesh_fields(&mut fields) else {
        return Ok(None);
    };
    let hidden = tree::field(mesh, h("initialSubmeshToHide"))
        .and_then(text)
        .unwrap_or_default();
    let joined = hidden
        .split_whitespace()
        .chain(names.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(" ");
    let Some(value) = string_value(&joined) else {
        return Ok(None);
    };
    tree::set_field(mesh, h("initialSubmeshToHide"), value);
    file.entries[at].body = tree::write_fields(&fields).map_err(bin_error)?;
    serialize_prop_file(&file).map(Some).map_err(bin_error)
}

#[cfg(test)]
mod tests;
