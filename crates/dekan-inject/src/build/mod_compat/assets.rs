use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AssetFormat {
    pub kind: &'static str,
    pub version: u32,
}

impl std::fmt::Display for AssetFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} v{}", self.kind, self.version)
    }
}

pub(super) fn u16_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from(u16::from_le_bytes(
        bytes.get(at..at + 2)?.try_into().ok()?,
    )))
}

pub(super) fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

#[must_use]
pub fn asset_format(head: &[u8]) -> Option<AssetFormat> {
    let format = |kind, version| Some(AssetFormat { kind, version });
    match head.get(..8)? {
        b"r3d2sklt" => return format("skl", u32_at(head, 8)?),
        b"r3d2anmd" => return format("anm", u32_at(head, 8)?),
        b"r3d2canm" => return format("anm-compressed", u32_at(head, 8)?),
        b"r3d2Mesh" => return format("scb", (u16_at(head, 8)? << 16) | u16_at(head, 10)?),
        _ => {}
    }
    match head.get(..4)? {
        [0x33, 0x22, 0x11, 0x00] => format("skn", (u16_at(head, 4)? << 16) | u16_at(head, 6)?),
        b"TEX\0" => format("tex", 0),
        b"DDS " => format("dds", 0),
        b"BKHD" => format("bnk", u32_at(head, 8)?),
        b"r3d2" => format("wpk", u32_at(head, 4)?),
        _ if u32_at(head, 4)? == SKL_FORMAT_TOKEN => format("skl", u32_at(head, 8)?),
        _ => None,
    }
}

pub(super) fn without_variant_suffix(stem: &str) -> Option<&str> {
    let (folder_end, file) = match stem.rfind('/') {
        Some(slash) => (slash + 1, &stem[slash + 1..]),
        None => (0, stem),
    };
    let dot = file.rfind('.')?;
    (dot > 0 && dot + 1 < file.len()).then(|| &stem[..folder_end + dot])
}

pub(super) fn asset_successors(path: &str) -> Vec<String> {
    if !path.is_ascii() {
        return Vec::new();
    }
    let Some((stem, extension)) = path.rsplit_once('.') else {
        return Vec::new();
    };
    let twin = ASSET_TWINS
        .iter()
        .find(|(from, _)| extension.eq_ignore_ascii_case(from))
        .map(|(_, to)| *to);
    let stripped = without_variant_suffix(stem);
    let mut out = Vec::new();
    if let Some(base) = stripped {
        out.push(format!("{base}.{extension}"));
    }
    if let Some(twin) = twin {
        out.push(format!("{stem}.{twin}"));
        if let Some(base) = stripped {
            out.push(format!("{base}.{twin}"));
        }
    }
    out
}

pub(super) fn is_asset_path(text: &str) -> bool {
    text.rsplit_once('.').is_some_and(|(_, extension)| {
        ASSET_EXTENSIONS
            .iter()
            .any(|known| extension.eq_ignore_ascii_case(known))
    })
}

pub(super) fn relink_assets(
    value: &mut Value,
    resolvable: &dyn Fn(&str) -> bool,
    in_game: &dyn Fn(&str) -> bool,
    relinks: &mut Vec<Relink>,
) {
    match value {
        Value::Raw { kind, bytes } if *kind == FIELD_STRING && bytes.len() >= 2 => {
            let Ok(text) = std::str::from_utf8(&bytes[2..]) else {
                return;
            };
            if !is_asset_path(text) || resolvable(text) {
                return;
            }
            let Some(new) = asset_successors(text).into_iter().find(|c| in_game(c)) else {
                return;
            };
            let Ok(len) = u16::try_from(new.len()) else {
                return;
            };
            relinks.push(Relink {
                from: text.to_owned(),
                to: new.clone(),
            });
            let mut replaced = len.to_le_bytes().to_vec();
            replaced.extend_from_slice(new.as_bytes());
            *bytes = replaced;
        }
        Value::Raw { .. } => {}
        Value::List { items, .. } => items
            .iter_mut()
            .for_each(|item| relink_assets(item, resolvable, in_game, relinks)),
        Value::Struct { fields, .. } => fields
            .iter_mut()
            .for_each(|field| relink_assets(&mut field.value, resolvable, in_game, relinks)),
        Value::Optional { value, .. } => {
            if let Some(inner) = value {
                relink_assets(inner, resolvable, in_game, relinks);
            }
        }
        Value::Map { entries, .. } => entries.iter_mut().for_each(|(key, value)| {
            relink_assets(key, resolvable, in_game, relinks);
            relink_assets(value, resolvable, in_game, relinks);
        }),
    }
}
