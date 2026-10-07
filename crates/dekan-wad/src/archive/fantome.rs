use std::io::{Read, Seek};

use zip::ZipArchive;

use crate::error::WadError;

pub fn wad_names_in_archive<R: Read + Seek>(
    reader: R,
) -> Result<std::collections::BTreeSet<String>, WadError> {
    let mut archive = ZipArchive::new(reader)
        .map_err(|e| WadError::InvalidFantome(format!("invalid zip archive: {e}")))?;
    let mut names = std::collections::BTreeSet::new();
    for i in 0..archive.len() {
        let file = archive
            .by_index(i)
            .map_err(|e| WadError::InvalidFantome(format!("zip read error: {e}")))?;
        if let Some(wad) = wad_name_in_path(&file.name().replace('\\', "/")) {
            names.insert(wad);
        }
    }
    Ok(names)
}

#[must_use]
pub fn wad_name_in_path(path: &str) -> Option<String> {
    let mut parts = path.split('/');
    if !parts.next()?.eq_ignore_ascii_case("WAD") {
        return None;
    }
    parts.find_map(|part| {
        let stem_len = part.to_ascii_lowercase().strip_suffix(".wad.client")?.len();
        (stem_len > 0).then(|| part[..stem_len].to_string())
    })
}

#[must_use]
pub fn wad_mount_alias(wad_name: &str) -> &str {
    wad_name.split('.').next().unwrap_or(wad_name)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModArchiveShape {
    pub manifest: bool,
    pub content: bool,
}

impl ModArchiveShape {}

pub fn mod_archive_shape<R: Read + Seek>(reader: R) -> Result<ModArchiveShape, WadError> {
    const MAX_MANIFEST: u64 = 1024 * 1024;

    let mut archive = ZipArchive::new(reader)
        .map_err(|e| WadError::InvalidFantome(format!("invalid zip archive: {e}")))?;
    let mut shape = ModArchiveShape {
        manifest: false,
        content: false,
    };
    for i in 0..archive.len() {
        let file = archive
            .by_index(i)
            .map_err(|e| WadError::InvalidFantome(format!("zip read error: {e}")))?;
        if file.is_dir() {
            continue;
        }
        let name = file.name().replace('\\', "/");
        let root = name.split('/').next().unwrap_or_default();
        if (root.eq_ignore_ascii_case("WAD") || root.eq_ignore_ascii_case("RAW"))
            && name.len() > root.len() + 1
        {
            shape.content = true;
        } else if name.eq_ignore_ascii_case("META/info.json") && file.size() <= MAX_MANIFEST {
            let mut text = String::new();
            let size = file.size();
            if file.take(size).read_to_string(&mut text).is_ok() {
                shape.manifest =
                    serde_json::from_str::<serde_json::Value>(text.trim_start_matches('\u{feff}'))
                        .is_ok_and(|v| v.is_object());
            }
        }
    }
    Ok(shape)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_the_targeted_wad_is_read_from_flat_nested_and_localized_paths() {
        assert_eq!(
            wad_name_in_path("WAD/Nasus.wad.client").as_deref(),
            Some("Nasus")
        );
        assert_eq!(
            wad_name_in_path("wad/Champions/Zed.wad.client/data/x.bin").as_deref(),
            Some("Zed")
        );
        assert_eq!(
            wad_name_in_path("WAD/Nasus.pt_BR.wad.client").as_deref(),
            Some("Nasus.pt_BR")
        );
        assert_eq!(wad_name_in_path("RAW/Nasus.wad.client"), None);
        assert_eq!(wad_name_in_path("WAD/.wad.client"), None);
        assert_eq!(wad_mount_alias("Nasus.pt_BR"), "Nasus");
        assert_eq!(wad_mount_alias("Nasus"), "Nasus");
    }
}
