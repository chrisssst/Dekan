use dekan_wad::hash::{content_checksum, wad_path_hash};

pub const RST_MASK: u64 = (1u64 << 38) - 1;
pub const TABLE_PATH: &str = "data/menu/en_us/lol.stringtable";
pub const RECORD_KEY_PREFIX: &str = "game_character_displayname_";
pub const RECORD_KEY_OWN_PREFIX: &str = "bllt_character_displayname_";

#[must_use]
pub fn table_hash() -> u64 {
    wad_path_hash(TABLE_PATH)
}

#[must_use]
pub fn key_hash(key: &str) -> u64 {
    let lower = key.to_ascii_lowercase();
    content_checksum(lower.as_bytes()) & RST_MASK
}

fn parse_header(table: &[u8]) -> Option<(usize, usize)> {
    if table.len() < 8 || &table[..3] != b"RST" || table[3] != 5 {
        return None;
    }
    let count = u32::from_le_bytes([table[4], table[5], table[6], table[7]]) as usize;
    let text_start = 8usize.checked_add(count.checked_mul(8)?)?;
    if text_start > table.len() {
        return None;
    }
    Some((count, text_start))
}

#[must_use]
pub fn read_text(table: &[u8], hash: u64) -> Option<String> {
    let (count, text_start) = parse_header(table)?;
    for i in 0..count {
        let pos = 8 + i * 8;
        let entry = u64::from_le_bytes([
            table[pos],
            table[pos + 1],
            table[pos + 2],
            table[pos + 3],
            table[pos + 4],
            table[pos + 5],
            table[pos + 6],
            table[pos + 7],
        ]);
        if (entry & RST_MASK) != hash {
            continue;
        }
        let offset = usize::try_from(entry >> 38).ok()?;
        let at = text_start.checked_add(offset)?;
        if at >= table.len() {
            return None;
        }
        let rel_end = table[at..].iter().position(|&b| b == 0)?;
        return Some(String::from_utf8_lossy(&table[at..at + rel_end]).into_owned());
    }
    None
}

#[must_use]
pub fn with_texts(table: &[u8], updates: &[(String, String)]) -> Option<Vec<u8>> {
    let (count, text_start) = parse_header(table)?;
    let mut entries = Vec::with_capacity(count + updates.len());
    for i in 0..count {
        let pos = 8 + i * 8;
        entries.push(u64::from_le_bytes([
            table[pos],
            table[pos + 1],
            table[pos + 2],
            table[pos + 3],
            table[pos + 4],
            table[pos + 5],
            table[pos + 6],
            table[pos + 7],
        ]));
    }
    let mut appended = Vec::new();
    let base_text_len = table.len().checked_sub(text_start)?;
    let mut added_new_key = false;

    for (key, value) in updates {
        let kh = key_hash(key);
        let offset = base_text_len.checked_add(appended.len())?;
        let offset_u64 = u64::try_from(offset).ok()?;
        let new_entry = kh | (offset_u64 << 38);
        appended.extend_from_slice(value.as_bytes());
        appended.push(0);

        if let Some(slot) = entries.iter_mut().find(|e| (**e & RST_MASK) == kh) {
            *slot = new_entry;
        } else {
            entries.push(new_entry);
            added_new_key = true;
        }
    }

    if added_new_key {
        entries.sort_unstable_by_key(|e| *e & RST_MASK);
    }

    let total_entries = u32::try_from(entries.len()).ok()?;
    let mut out = Vec::with_capacity(8 + entries.len() * 8 + base_text_len + appended.len());
    out.extend_from_slice(&table[..4]);
    out.extend_from_slice(&total_entries.to_le_bytes());
    for e in entries {
        out.extend_from_slice(&e.to_le_bytes());
    }
    out.extend_from_slice(&table[text_start..]);
    out.extend_from_slice(&appended);
    Some(out)
}

#[must_use]
pub fn patch_record_key(record: &[u8], alias: &str) -> Option<Vec<u8>> {
    let target = format!("{RECORD_KEY_PREFIX}{alias}");
    let target_bytes = target.as_bytes();
    let len_u16 = u16::try_from(target_bytes.len()).ok()?;
    let len_bytes = len_u16.to_le_bytes();
    let own_prefix = RECORD_KEY_OWN_PREFIX.as_bytes();
    let pattern_len = 2usize.checked_add(target_bytes.len())?;
    if record.len() < pattern_len {
        return None;
    }

    let mut out = record.to_vec();
    let mut found = false;
    let mut i = 0usize;
    let limit = record.len() - pattern_len;
    while i <= limit {
        if out[i] == len_bytes[0]
            && out[i + 1] == len_bytes[1]
            && out[i + 2..i + pattern_len].eq_ignore_ascii_case(target_bytes)
        {
            out[i + 2..i + 2 + own_prefix.len()].copy_from_slice(own_prefix);
            found = true;
            i += pattern_len;
        } else {
            i += 1;
        }
    }
    found.then_some(out)
}
