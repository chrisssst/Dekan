use crate::binary::Reader;
use crate::error::ClassicError;

const MAGIC: u32 = 0x0011_2233;
const MAJOR: u16 = 4;
const NAME: usize = 64;
const SUBMESH: usize = NAME + 16;
const SUBMESHES_AT: usize = 12;
const COUNTS: usize = 20;
const BOUNDS: usize = 10;
pub(crate) const BASIC_VERTEX: usize = 52;
pub(crate) const COLOR_VERTEX: usize = 56;
pub(crate) const TANGENT_VERTEX: usize = 72;
pub(crate) const INFLUENCES_AT: usize = 12;
pub(crate) const COLOR_AT: usize = 52;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Submesh {
    pub name: String,
    pub first_vertex: u32,
    pub vertex_count: u32,
    pub first_index: u32,
    pub index_count: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SkinnedMesh {
    pub minor: u16,
    pub flags: u32,
    pub vertex_type: u32,
    pub vertex_size: usize,
    pub bounds: [f32; BOUNDS],
    pub submeshes: Vec<Submesh>,
    pub indices: Vec<u16>,
    pub vertices: Vec<u8>,
    pub trailer: Vec<u8>,
}

impl SkinnedMesh {
    #[must_use]
    pub fn vertex_count(&self) -> usize {
        self.vertices
            .len()
            .checked_div(self.vertex_size)
            .unwrap_or(0)
    }

    #[must_use]
    pub fn vertex(&self, index: usize) -> Option<&[u8]> {
        let start = index.checked_mul(self.vertex_size)?;
        self.vertices
            .get(start..start.checked_add(self.vertex_size)?)
    }
}

fn corrupt(what: String) -> ClassicError {
    ClassicError::Mesh(format!("skinned mesh: {what}"))
}

pub fn parse(bytes: &[u8]) -> Result<SkinnedMesh, ClassicError> {
    let r = Reader::new(bytes, corrupt);
    if r.u32(0)? != MAGIC {
        return Err(r.fail("not a skinned mesh"));
    }
    let major = r.u16(4)?;
    if major != MAJOR {
        return Err(r.fail(format!("version {major} is not supported")));
    }
    let count = r.u32(8)? as usize;
    let table = Reader::new(
        r.section(SUBMESHES_AT, count.saturating_mul(SUBMESH))?,
        corrupt,
    );
    let mut submeshes = Vec::with_capacity(count);
    for at in (0..count).map(|i| i * SUBMESH) {
        let raw = table.section(at, NAME)?;
        let end = raw.iter().position(|b| *b == 0).unwrap_or(NAME);
        submeshes.push(Submesh {
            name: String::from_utf8(raw[..end].to_vec())
                .map_err(|_| r.fail("a submesh name is not UTF-8"))?,
            first_vertex: table.u32(at + NAME)?,
            vertex_count: table.u32(at + NAME + 4)?,
            first_index: table.u32(at + NAME + 8)?,
            index_count: table.u32(at + NAME + 12)?,
        });
    }
    let at = SUBMESHES_AT + table.bytes.len();
    let index_count = r.u32(at + 4)? as usize;
    let vertex_count = r.u32(at + 8)? as usize;
    let vertex_size = r.u32(at + 12)? as usize;
    if !matches!(vertex_size, BASIC_VERTEX | COLOR_VERTEX | TANGENT_VERTEX) {
        return Err(r.fail(format!("vertex size {vertex_size} is not supported")));
    }
    let mut bounds = [0f32; BOUNDS];
    for (i, slot) in bounds.iter_mut().enumerate() {
        *slot = r.f32(at + COUNTS + i * 4)?;
    }
    let indices_at = at + COUNTS + BOUNDS * 4;
    let indices: Vec<u16> = r
        .section(indices_at, index_count.saturating_mul(2))?
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    let vertices_at = indices_at + indices.len() * 2;
    let vertices = r.section(vertices_at, vertex_count.saturating_mul(vertex_size))?;
    if let Some(bad) = indices.iter().find(|i| usize::from(**i) >= vertex_count) {
        return Err(r.fail(format!("index {bad} names a vertex past {vertex_count}")));
    }
    for part in &submeshes {
        let vertices_end = u64::from(part.first_vertex) + u64::from(part.vertex_count);
        let indices_end = u64::from(part.first_index) + u64::from(part.index_count);
        if vertices_end > vertex_count as u64 || indices_end > index_count as u64 {
            return Err(r.fail(format!("submesh '{}' runs past the mesh", part.name)));
        }
    }
    Ok(SkinnedMesh {
        minor: r.u16(6)?,
        flags: r.u32(at)?,
        vertex_type: r.u32(at + 16)?,
        vertex_size,
        bounds,
        submeshes,
        indices,
        vertices: vertices.to_vec(),
        trailer: bytes[vertices_at + vertices.len()..].to_vec(),
    })
}

fn put_u32(out: &mut Vec<u8>, value: usize) -> Result<(), ClassicError> {
    let value = u32::try_from(value).map_err(|_| corrupt(format!("{value} does not fit")))?;
    out.extend_from_slice(&value.to_le_bytes());
    Ok(())
}

pub fn write(mesh: &SkinnedMesh) -> Result<Vec<u8>, ClassicError> {
    if mesh.vertex_size == 0 || mesh.vertices.len() % mesh.vertex_size != 0 {
        return Err(corrupt(
            "vertex data is not a whole number of vertices".into(),
        ));
    }
    let mut out = Vec::with_capacity(
        SUBMESHES_AT
            + mesh.submeshes.len() * SUBMESH
            + COUNTS
            + BOUNDS * 4
            + mesh.indices.len() * 2
            + mesh.vertices.len(),
    );
    out.extend_from_slice(&MAGIC.to_le_bytes());
    out.extend_from_slice(&MAJOR.to_le_bytes());
    out.extend_from_slice(&mesh.minor.to_le_bytes());
    put_u32(&mut out, mesh.submeshes.len())?;
    for part in &mesh.submeshes {
        if part.name.len() >= NAME {
            return Err(corrupt(format!("submesh name '{}' is too long", part.name)));
        }
        let mut raw = [0u8; NAME];
        raw[..part.name.len()].copy_from_slice(part.name.as_bytes());
        out.extend_from_slice(&raw);
        for value in [
            part.first_vertex,
            part.vertex_count,
            part.first_index,
            part.index_count,
        ] {
            out.extend_from_slice(&value.to_le_bytes());
        }
    }
    out.extend_from_slice(&mesh.flags.to_le_bytes());
    put_u32(&mut out, mesh.indices.len())?;
    put_u32(&mut out, mesh.vertex_count())?;
    put_u32(&mut out, mesh.vertex_size)?;
    out.extend_from_slice(&mesh.vertex_type.to_le_bytes());
    mesh.bounds
        .iter()
        .for_each(|v| out.extend_from_slice(&v.to_le_bytes()));
    mesh.indices
        .iter()
        .for_each(|i| out.extend_from_slice(&i.to_le_bytes()));
    out.extend_from_slice(&mesh.vertices);
    out.extend_from_slice(&mesh.trailer);
    Ok(out)
}

#[cfg(test)]
pub(crate) mod tests;
