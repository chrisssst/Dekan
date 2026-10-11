use super::*;

pub(super) fn collect_files(value: &Value, out: &mut Vec<(String, u64)>) {
    match value {
        Value::Raw { kind: 16, bytes } if bytes.len() >= 2 => {
            let text = String::from_utf8_lossy(&bytes[2..]).to_string();
            let lower = text.to_ascii_lowercase();
            if ASSET_EXTENSIONS.iter().any(|ext| lower.ends_with(ext)) {
                out.push((text, wad_path_hash(&lower)));
            }
        }
        Value::Raw { kind: 18, bytes } if bytes.len() == 8 => {
            let mut raw = [0u8; 8];
            raw.copy_from_slice(bytes);
            let hash = u64::from_le_bytes(raw);
            if hash != 0 {
                out.push((format!("file {hash:016x}"), hash));
            }
        }
        Value::Raw { .. } => {}
        Value::List { items, .. } => items.iter().for_each(|i| collect_files(i, out)),
        Value::Struct { fields, .. } => fields.iter().for_each(|f| collect_files(&f.value, out)),
        Value::Optional { value, .. } => {
            if let Some(inner) = value {
                collect_files(inner, out);
            }
        }
        Value::Map { entries, .. } => entries.iter().for_each(|(k, v)| {
            collect_files(k, out);
            collect_files(v, out);
        }),
    }
}

pub(super) fn is_asset_text(bytes: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(bytes.get(2..)?).ok()?;
    let lower = text.to_ascii_lowercase();
    ASSET_EXTENSIONS
        .iter()
        .any(|ext| lower.ends_with(ext))
        .then(|| text.to_owned())
}

pub(super) fn same_value(a: &Value, b: &Value, assets: &mut Vec<(String, String)>) -> bool {
    match (a, b) {
        (
            Value::Raw {
                kind: ka,
                bytes: ba,
            },
            Value::Raw {
                kind: kb,
                bytes: bb,
            },
        ) => {
            if ka != kb {
                return false;
            }
            if ba == bb {
                return true;
            }
            match (*ka == 16).then(|| (is_asset_text(ba), is_asset_text(bb))) {
                Some((Some(old), Some(new))) => {
                    assets.push((old, new));
                    true
                }
                _ => false,
            }
        }
        (
            Value::List {
                kind: ka,
                element: ea,
                items: ia,
            },
            Value::List {
                kind: kb,
                element: eb,
                items: ib,
            },
        ) => {
            ka == kb
                && ea == eb
                && ia.len() == ib.len()
                && ia.iter().zip(ib).all(|(x, y)| same_value(x, y, assets))
        }
        (
            Value::Struct {
                kind: ka,
                class: ca,
                fields: fa,
            },
            Value::Struct {
                kind: kb,
                class: cb,
                fields: fb,
            },
        ) => ka == kb && ca == cb && same_but_asset_paths(fa, fb, assets),
        (
            Value::Optional {
                inner: ia,
                value: va,
            },
            Value::Optional {
                inner: ib,
                value: vb,
            },
        ) => {
            ia == ib
                && match (va, vb) {
                    (Some(x), Some(y)) => same_value(x, y, assets),
                    (None, None) => true,
                    _ => false,
                }
        }
        (
            Value::Map {
                key: ka,
                value: va,
                entries: ea,
            },
            Value::Map {
                key: kb,
                value: vb,
                entries: eb,
            },
        ) => {
            ka == kb
                && va == vb
                && ea.len() == eb.len()
                && ea.iter().zip(eb).all(|((k1, v1), (k2, v2))| {
                    same_value(k1, k2, assets) && same_value(v1, v2, assets)
                })
        }
        _ => false,
    }
}

pub(super) fn same_but_asset_paths(
    a: &[Field],
    b: &[Field],
    assets: &mut Vec<(String, String)>,
) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(x, y)| x.name == y.name && same_value(&x.value, &y.value, assets))
}

pub(super) fn prop_files(prop: &PropFile) -> Vec<(String, u64)> {
    let mut files = Vec::new();
    for entry in &prop.entries {
        if let Ok(fields) = tree::parse_fields(&entry.body) {
            fields
                .iter()
                .for_each(|f| collect_files(&f.value, &mut files));
        }
    }
    files
}

pub(super) fn closure_keys(
    prop: &PropFile,
    own: &WadFile,
    game: &mut Game,
) -> (HashSet<u32>, usize) {
    let mut keys: HashSet<u32> = prop.entries.iter().map(|e| e.key_hash).collect();
    let mut seen = HashSet::new();
    let mut queue = prop.links.clone();
    let mut missing = 0;
    while let Some(link) = queue.pop() {
        let hash = wad_path_hash(&link.to_ascii_lowercase());
        if !seen.insert(hash) {
            continue;
        }
        let bytes = own.read(hash).ok().flatten().or_else(|| game.read(hash));
        let Some(linked) = bytes.and_then(|b| parse_prop_file(&b).ok()) else {
            missing += 1;
            continue;
        };
        keys.extend(linked.entries.iter().map(|e| e.key_hash));
        queue.extend(linked.links);
    }
    (keys, missing)
}

pub(super) const FIELD_LINK: u8 = 0x84;

pub(super) fn collect_links(value: &Value, out: &mut Vec<u32>) {
    match value {
        Value::Raw { kind, bytes } if *kind == FIELD_LINK && bytes.len() == 4 => {
            out.push(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]));
        }
        Value::Raw { .. } => {}
        Value::List { items, .. } => items.iter().for_each(|i| collect_links(i, out)),
        Value::Struct { fields, .. } => fields.iter().for_each(|f| collect_links(&f.value, out)),
        Value::Optional { value, .. } => {
            if let Some(inner) = value {
                collect_links(inner, out);
            }
        }
        Value::Map { entries, .. } => entries.iter().for_each(|(k, v)| {
            collect_links(k, out);
            collect_links(v, out);
        }),
    }
}

pub(super) fn unresolved_refs(prop: &PropFile, keys: &HashSet<u32>) -> BTreeSet<u32> {
    let mut links = Vec::new();
    for entry in &prop.entries {
        if let Ok(fields) = tree::parse_fields(&entry.body) {
            fields
                .iter()
                .for_each(|f| collect_links(&f.value, &mut links));
        }
    }
    links
        .into_iter()
        .filter(|r| *r != 0 && !keys.contains(r))
        .collect()
}
