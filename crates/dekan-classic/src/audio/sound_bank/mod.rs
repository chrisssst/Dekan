use crate::binary::{Reader, u32_le};
use crate::error::ClassicError;

const HEADER: &[u8; 4] = b"BKHD";
const HIERARCHY: &[u8; 4] = b"HIRC";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub tag: [u8; 4],
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Object {
    pub kind: u8,
    pub id: u32,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoundBank {
    pub sections: Vec<Section>,
}

#[must_use]
pub fn wwise_id(name: &str) -> u32 {
    name.bytes().fold(0x811C_9DC5u32, |hash, byte| {
        hash.wrapping_mul(0x0100_0193) ^ u32::from(byte.to_ascii_lowercase())
    })
}

fn corrupt(what: String) -> ClassicError {
    ClassicError::Audio(what)
}

pub fn parse(bytes: &[u8]) -> Result<SoundBank, ClassicError> {
    let r = Reader::new(bytes, corrupt);
    if r.take::<4>(0)? != *HEADER {
        return Err(r.fail("not a Wwise sound bank"));
    }
    let mut sections = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let size = r.u32(at + 4)? as usize;
        sections.push(Section {
            tag: r.take(at)?,
            data: r.section(at + 8, size)?.to_vec(),
        });
        at += 8 + size;
    }
    let bank = SoundBank { sections };
    bank.objects()?;
    Ok(bank)
}

impl SoundBank {
    #[must_use]
    pub fn version(&self) -> Option<u32> {
        self.sections
            .iter()
            .find(|s| &s.tag == HEADER)
            .and_then(|s| u32_le(&s.data, 0))
    }

    pub fn objects(&self) -> Result<Vec<Object>, ClassicError> {
        let Some(hierarchy) = self.sections.iter().find(|s| &s.tag == HIERARCHY) else {
            return Ok(Vec::new());
        };
        let r = Reader::new(&hierarchy.data, corrupt);
        let count = r.u32(0)? as usize;
        let mut objects = Vec::with_capacity(count.min(r.bytes.len() / 9));
        let mut at = 4;
        for _ in 0..count {
            let [kind] = r.take(at)?;
            let size = r.u32(at + 1)? as usize;
            if size < 4 {
                return Err(r.fail(format!("object at {at} is shorter than its id")));
            }
            objects.push(Object {
                kind,
                id: r.u32(at + 5)?,
                payload: r.section(at + 9, size - 4)?.to_vec(),
            });
            at += 5 + size;
        }
        if at != r.bytes.len() {
            return Err(r.fail("the hierarchy has bytes after its last object"));
        }
        Ok(objects)
    }

    pub fn append(&mut self, added: &[Object]) -> Result<(), ClassicError> {
        let hierarchy = self
            .sections
            .iter_mut()
            .find(|s| &s.tag == HIERARCHY)
            .ok_or_else(|| corrupt("the bank has no hierarchy to add to".into()))?;
        let count = Reader::new(&hierarchy.data, corrupt).u32(0)?;
        let count = count
            .checked_add(
                u32::try_from(added.len()).map_err(|_| corrupt("too many objects".into()))?,
            )
            .ok_or_else(|| corrupt("object count overflow".into()))?;
        hierarchy.data[..4].copy_from_slice(&count.to_le_bytes());
        for object in added {
            let size = u32::try_from(object.payload.len() + 4)
                .map_err(|_| corrupt("object too large".into()))?;
            hierarchy.data.push(object.kind);
            hierarchy.data.extend_from_slice(&size.to_le_bytes());
            hierarchy.data.extend_from_slice(&object.id.to_le_bytes());
            hierarchy.data.extend_from_slice(&object.payload);
        }
        Ok(())
    }

    pub fn write(&self) -> Result<Vec<u8>, ClassicError> {
        let mut out = Vec::new();
        for section in &self.sections {
            let size = u32::try_from(section.data.len())
                .map_err(|_| corrupt("section too large".into()))?;
            out.extend_from_slice(&section.tag);
            out.extend_from_slice(&size.to_le_bytes());
            out.extend_from_slice(&section.data);
        }
        Ok(out)
    }
}

#[cfg(test)]
pub(crate) mod tests;
