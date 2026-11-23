use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use dekan_wad::hash::wad_path_hash;
use dekan_wad::wad::WadFile;

use crate::binary::u32_le;
use crate::error::ClassicError;
use crate::sound_bank::{self, Object, Section, SoundBank, wwise_id};

pub(super) const HIRC_ACTION: u8 = 3;
pub(super) const HIRC_EVENT: u8 = 4;
const LAST_U32_ACTION_COUNT_VERSION: u32 = 122;

fn read_var_uint(data: &[u8]) -> Option<(usize, usize)> {
    let mut value = 0usize;
    for (index, &byte) in data.iter().take(5).enumerate() {
        value = (value << 7) | usize::from(byte & 0x7F);
        if (byte & 0x80) == 0 {
            return Some((value, index + 1));
        }
    }
    None
}

fn write_var_uint(mut value: usize) -> Vec<u8> {
    let mut groups = vec![(value & 0x7F) as u8];
    value >>= 7;
    while value > 0 {
        groups.push(((value & 0x7F) as u8) | 0x80);
        value >>= 7;
    }
    groups.reverse();
    groups
}

pub(super) fn decode_actions(body: &[u8], version: u32) -> Option<Vec<u32>> {
    let (count, pos) = if version <= LAST_U32_ACTION_COUNT_VERSION {
        (u32_le(body, 0)? as usize, 4usize)
    } else {
        read_var_uint(body)?
    };
    let expected = count.checked_mul(4)?.checked_add(pos)?;
    if body.len() != expected {
        return None;
    }
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        out.push(u32_le(body, pos + i * 4)?);
    }
    Some(out)
}

pub(super) fn encode_actions(actions: &[u32], version: u32) -> Vec<u8> {
    let mut out = if version <= LAST_U32_ACTION_COUNT_VERSION {
        (actions.len() as u32).to_le_bytes().to_vec()
    } else {
        write_var_uint(actions.len())
    };
    out.reserve(actions.len() * 4);
    for &id in actions {
        out.extend_from_slice(&id.to_le_bytes());
    }
    out
}

fn build_hirc_section(objects: &[Object]) -> Result<Vec<u8>, ClassicError> {
    let count = u32::try_from(objects.len())
        .map_err(|_| ClassicError::Audio("too many HIRC objects".into()))?;
    let mut data = count.to_le_bytes().to_vec();
    for obj in objects {
        let size = u32::try_from(obj.payload.len() + 4)
            .map_err(|_| ClassicError::Audio("HIRC object too large".into()))?;
        data.push(obj.kind);
        data.extend_from_slice(&size.to_le_bytes());
        data.extend_from_slice(&obj.id.to_le_bytes());
        data.extend_from_slice(&obj.payload);
    }
    Ok(data)
}

pub(super) struct ParsedEventBank {
    pub version: u32,
    sections: Vec<Section>,
    objects: Vec<Object>,
    event_index: HashMap<u32, usize>,
    pub modified: bool,
}

impl ParsedEventBank {
    pub fn parse_bytes(bytes: &[u8]) -> Result<Self, ClassicError> {
        let (bank, objects) = sound_bank::parse_with_objects(bytes)?;
        let version = bank
            .version()
            .ok_or_else(|| ClassicError::Audio("bank has no BKHD chunk".into()))?;
        let mut event_index = HashMap::new();
        for (idx, obj) in objects.iter().enumerate() {
            if obj.kind == HIRC_EVENT {
                event_index.entry(obj.id).or_insert(idx);
            }
        }
        Ok(Self {
            version,
            sections: bank.sections,
            objects,
            event_index,
            modified: false,
        })
    }

    pub fn event_ids(&self) -> Vec<u32> {
        self.event_index.keys().copied().collect()
    }

    pub fn has_event(&self, event_id: u32) -> bool {
        self.event_index.contains_key(&event_id)
    }

    pub fn event_actions(&self, event_id: u32) -> Option<Vec<u32>> {
        let &idx = self.event_index.get(&event_id)?;
        let obj = self.objects.get(idx)?;
        decode_actions(&obj.payload, self.version)
    }

    pub fn set_or_add_event(&mut self, event_id: u32, actions: &[u32]) {
        let body = encode_actions(actions, self.version);
        if let Some(&idx) = self.event_index.get(&event_id) {
            if let Some(obj) = self.objects.get_mut(idx) {
                if obj.payload != body {
                    obj.payload = body;
                    self.modified = true;
                }
            }
        } else {
            let idx = self.objects.len();
            self.objects.push(Object {
                kind: HIRC_EVENT,
                id: event_id,
                payload: body,
            });
            self.event_index.insert(event_id, idx);
            self.modified = true;
        }
    }

    pub fn remove_event(&mut self, event_id: u32) -> bool {
        if self.event_index.remove(&event_id).is_none() {
            return false;
        }
        self.objects
            .retain(|o| !(o.kind == HIRC_EVENT && o.id == event_id));
        self.event_index.clear();
        for (idx, obj) in self.objects.iter().enumerate() {
            if obj.kind == HIRC_EVENT {
                self.event_index.entry(obj.id).or_insert(idx);
            }
        }
        self.modified = true;
        true
    }

    pub fn write(&self) -> Result<Vec<u8>, ClassicError> {
        let mut sections = self.sections.clone();
        let hirc_bytes = build_hirc_section(&self.objects)?;
        if let Some(sec) = sections.iter_mut().find(|s| &s.tag == b"HIRC") {
            sec.data = hirc_bytes;
        } else {
            sections.push(Section {
                tag: *b"HIRC",
                data: hirc_bytes,
            });
        }
        SoundBank { sections }.write()
    }

    pub fn extract_sidecar(
        &self,
        bank_id: u32,
        pairs: &[(u32, u32)],
    ) -> Result<Vec<u8>, ClassicError> {
        let bkhd = self
            .sections
            .iter()
            .find(|s| &s.tag == b"BKHD")
            .ok_or_else(|| ClassicError::Audio("bank has no BKHD chunk".into()))?;
        if bkhd.data.len() < 12 {
            return Err(ClassicError::Audio(
                "BKHD chunk shorter than 12 bytes".into(),
            ));
        }
        let mut header = bkhd.data.clone();
        header[4..8].copy_from_slice(&bank_id.to_le_bytes());
        header[8..12].copy_from_slice(&wwise_id("SFX").to_le_bytes());

        let actions_by_id: BTreeMap<u32, &Object> = self
            .objects
            .iter()
            .filter(|o| o.kind == HIRC_ACTION)
            .map(|o| (o.id, o))
            .collect();

        let mut seen_actions = BTreeSet::new();
        let mut seen_events = BTreeSet::new();
        let mut actions_out = Vec::new();
        let mut events_out = Vec::new();

        for &(alias_id, source_id) in pairs {
            if !seen_events.insert(alias_id) {
                continue;
            }
            let Some(action_ids) = self.event_actions(source_id) else {
                continue;
            };
            let mut objs = Vec::with_capacity(action_ids.len());
            let mut complete = true;
            for aid in &action_ids {
                if let Some(obj) = actions_by_id.get(aid) {
                    objs.push(*obj);
                } else {
                    complete = false;
                    break;
                }
            }
            if !complete {
                continue;
            }
            for obj in objs {
                if seen_actions.insert(obj.id) {
                    actions_out.push(obj.clone());
                }
            }
            events_out.push(Object {
                kind: HIRC_EVENT,
                id: alias_id,
                payload: encode_actions(&action_ids, self.version),
            });
        }

        if events_out.is_empty() {
            return Err(ClassicError::Audio(
                "no matching VO events extracted for sidecar".into(),
            ));
        }
        actions_out.extend(events_out);
        let hirc_data = build_hirc_section(&actions_out)?;
        SoundBank {
            sections: vec![
                Section {
                    tag: *b"BKHD",
                    data: header,
                },
                Section {
                    tag: *b"HIRC",
                    data: hirc_data,
                },
            ],
        }
        .write()
    }
}

pub(super) struct LoadedBank {
    pub path: String,
    pub localized: bool,
    pub bank: ParsedEventBank,
}

pub(super) fn is_locale_segment(seg: &str) -> bool {
    let b = seg.as_bytes();
    b.len() == 5
        && b[2] == b'_'
        && b[0].is_ascii_alphabetic()
        && b[1].is_ascii_alphabetic()
        && b[3].is_ascii_alphabetic()
        && b[4].is_ascii_alphabetic()
}

pub(super) fn sidecar_path(source: &str) -> String {
    let mut parts: Vec<&str> = source
        .split('/')
        .filter(|s| !is_locale_segment(s))
        .collect();
    let file_name = parts.pop().unwrap_or("");
    let stem = file_name
        .strip_suffix("_events.bnk")
        .or_else(|| file_name.strip_suffix(".bnk"))
        .unwrap_or(file_name);
    let mut prefix = parts.join("/");
    if !prefix.is_empty() {
        prefix.push('/');
    }
    format!("{prefix}{stem}_basetag_events.bnk")
}

pub(super) struct LocaleWadSlot {
    path: PathBuf,
    opened: Option<Option<WadFile>>,
}

pub(super) fn load_bank(
    main_wad: &WadFile,
    wad_root: &Path,
    locale_wads: &mut Option<Vec<LocaleWadSlot>>,
    bank_path: &str,
) -> Option<LoadedBank> {
    let lower = bank_path.to_ascii_lowercase();
    let local = wad_root.join(&lower);
    if local.is_file() {
        let bytes = std::fs::read(&local).ok()?;
        let bank = ParsedEventBank::parse_bytes(&bytes).ok()?;
        return Some(LoadedBank {
            path: bank_path.to_owned(),
            localized: false,
            bank,
        });
    }
    let hash = wad_path_hash(&lower);
    if let Ok(Some(bytes)) = main_wad.read(hash) {
        let bank = ParsedEventBank::parse_bytes(&bytes).ok()?;
        return Some(LoadedBank {
            path: bank_path.to_owned(),
            localized: false,
            bank,
        });
    }
    let slots = locale_wads.get_or_insert_with(|| discover_locale_wad_paths(main_wad.path()));
    for slot in slots.iter_mut() {
        let wad_opt = slot
            .opened
            .get_or_insert_with(|| WadFile::open(&slot.path).ok());
        if let Some(lwad) = wad_opt.as_ref()
            && let Ok(Some(bytes)) = lwad.read(hash)
            && let Ok(bank) = ParsedEventBank::parse_bytes(&bytes)
        {
            return Some(LoadedBank {
                path: bank_path.to_owned(),
                localized: true,
                bank,
            });
        }
    }
    None
}

fn discover_locale_wad_paths(main_path: &Path) -> Vec<LocaleWadSlot> {
    let Some(dir) = main_path.parent() else {
        return Vec::new();
    };
    let Some(file_name) = main_path.file_name().and_then(|s| s.to_str()) else {
        return Vec::new();
    };
    let Some(stem) = file_name.strip_suffix(".wad.client") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    let mut paths: Vec<_> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    for p in paths {
        let Some(name) = p.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        let Some(rest) = name.strip_prefix(stem).and_then(|r| r.strip_prefix('.')) else {
            continue;
        };
        let Some(locale) = rest.strip_suffix(".wad.client") else {
            continue;
        };
        if is_locale_segment(locale) {
            out.push(LocaleWadSlot {
                path: p,
                opened: None,
            });
        }
    }
    out
}
