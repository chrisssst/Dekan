use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use dekan_wad::hash::{prop_key_hash, wad_path_hash};
use dekan_wad::prop::{
    PropEntry, PropFile, field_value, parse_prop_file, remap_references, serialize_prop_file,
    set_int_field,
};
use dekan_wad::wad::WadFile;
use tracing::{debug, info, warn};

use crate::builder::CLASSIC_DEFAULT_SLOTS;
use crate::error::ClassicError;

pub const CLASSIC_MOD_PREFIX: &str = "classic_";

const SKIN_CLASSIFICATION_FIELD: &str = "skinClassification";
const SKIN_PARENT_FIELD: &str = "skinParent";

#[must_use]
pub fn is_safe_alias(alias: &str) -> bool {
    !alias.is_empty()
        && alias
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

#[must_use]
pub fn skin_number(skin_or_chroma_id: u32) -> u32 {
    crate::builder::normalize_skin_id(skin_or_chroma_id) % 1000
}

#[must_use]
pub fn main_character(alias: &str) -> String {
    format!("jade_{}", alias.to_ascii_lowercase())
}

fn character_bin(character: &str) -> String {
    format!("data/characters/{character}/{character}.bin")
}

fn skin_bin(character: &str, skin: u32) -> String {
    format!("data/characters/{character}/skins/skin{skin}.bin")
}

fn animation_bin(character: &str, skin: u32) -> String {
    format!("data/characters/{character}/animations/skin{skin}.bin")
}

const SKIN_DATA_CLASS: u32 = 0x9b67_e9f6;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SlotIdentity {
    pub classification: Option<u32>,
    pub parent: u32,
}

pub fn slot_identity(slot_bin: &[u8]) -> Result<SlotIdentity, ClassicError> {
    let parsed = parse_prop_file(slot_bin).map_err(|e| ClassicError::Bin(e.to_string()))?;
    let skin = parsed
        .entries
        .iter()
        .find(|e| e.class_hash == SKIN_DATA_CLASS)
        .ok_or_else(|| ClassicError::Bin("no skin object in the slot bin".into()))?;
    let read = |name: &str| {
        field_value(&skin.body, &[prop_key_hash(name)])
            .map(|v| v.and_then(|v| v.as_u32()))
            .map_err(|e| ClassicError::Bin(e.to_string()))
    };
    Ok(SlotIdentity {
        classification: read(SKIN_CLASSIFICATION_FIELD)?,
        parent: read(SKIN_PARENT_FIELD)?.unwrap_or(0),
    })
}

mod characters;
mod classic;
mod retarget;
mod standard;

pub use characters::*;
pub use classic::*;
pub use retarget::*;
pub use standard::*;

pub(crate) fn identity_at(wad: &WadFile, character: &str, slot: u32) -> Option<SlotIdentity> {
    let path = skin_bin(character, slot);
    match wad.read(wad_path_hash(&path)) {
        Ok(Some(bin)) => match slot_identity(&bin) {
            Ok(identity) => Some(identity),
            Err(e) => {
                warn!(character, slot, error = %e, "Slot identity unreadable; the source skin keeps its own");
                None
            }
        },
        Ok(None) => None,
        Err(e) => {
            warn!(character, slot, error = %e, "Slot bin unreadable; the source skin keeps its own identity");
            None
        }
    }
}

pub(crate) fn remove_if_present(dir: &Path) -> Result<(), ClassicError> {
    match std::fs::remove_dir_all(dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
#[path = "generator_tests.rs"]
mod tests;
