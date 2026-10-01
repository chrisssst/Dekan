use std::fs::File;
use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use zip::ZipArchive;

use tracing::{debug, warn};

use crate::error::WadError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct FantomeManifest {
    pub name: String,

    #[serde(default)]
    pub author: String,

    #[serde(default)]
    pub version: String,

    #[serde(default)]
    pub description: Option<String>,

    #[serde(default)]
    pub champ_id: Option<u32>,
}

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

impl ModArchiveShape {
    #[must_use]
    pub fn is_mod(&self) -> bool {
        self.manifest && self.content
    }
}

pub fn mod_archive_shape<R: Read + Seek>(reader: R) -> Result<ModArchiveShape, WadError> {
    const MAX_MANIFEST: u64 = 1024 * 1024;

    let mut archive = ZipArchive::new(reader)
        .map_err(|e| WadError::InvalidFantome(format!("invalid zip archive: {e}")))?;
    let mut shape = ModArchiveShape {
        manifest: false,
        content: false,
    };
    for i in 0..archive.len() {
        let mut file = archive
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
            if file.read_to_string(&mut text).is_ok() {
                shape.manifest =
                    serde_json::from_str::<serde_json::Value>(text.trim_start_matches('\u{feff}'))
                        .is_ok_and(|v| v.is_object());
            }
        }
    }
    Ok(shape)
}

pub fn read_fantome_manifest<R: Read + Seek>(reader: R) -> Result<FantomeManifest, WadError> {
    let mut archive = ZipArchive::new(reader)
        .map_err(|e| WadError::InvalidFantome(format!("invalid zip archive: {e}")))?;

    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| WadError::InvalidFantome(format!("zip read error: {e}")))?;

        let name = file.name().replace('\\', "/");
        if name.eq_ignore_ascii_case("META/info.json") {
            let mut content = String::new();
            file.read_to_string(&mut content)
                .map_err(|e| WadError::InvalidFantome(format!("failed to read info.json: {e}")))?;

            let manifest: FantomeManifest = serde_json::from_str(&content).map_err(|e| {
                WadError::InvalidFantome(format!("corrupt info.json manifest: {e}"))
            })?;

            debug!(
                name = %manifest.name,
                author = %manifest.author,
                version = %manifest.version,
                "Fantome manifest read"
            );
            return Ok(manifest);
        }
    }

    warn!(
        entries = archive.len(),
        "Fantome package has no META/info.json manifest"
    );
    Err(WadError::InvalidFantome(
        "missing 'META/info.json' manifest inside .fantome package".into(),
    ))
}

pub fn extract_fantome_wads<R: Read + Seek>(
    reader: R,
    dest_dir: &Path,
) -> Result<Vec<PathBuf>, WadError> {
    if !dest_dir.exists() {
        std::fs::create_dir_all(dest_dir).map_err(|e| {
            WadError::InvalidFantome(format!("failed to create destination dir: {e}"))
        })?;
    }

    let mut archive = ZipArchive::new(reader)
        .map_err(|e| WadError::InvalidFantome(format!("invalid zip archive: {e}")))?;

    let mut extracted_wads = Vec::new();

    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| WadError::InvalidFantome(format!("zip read error: {e}")))?;

        let raw_name = file.name().replace('\\', "/");

        if (raw_name.starts_with("WAD/") || raw_name.starts_with("wad/")) && !file.is_dir() {
            let file_name = Path::new(&raw_name).file_name().ok_or_else(|| {
                WadError::InvalidFantome(format!("invalid WAD file path '{raw_name}'"))
            })?;

            let out_path = dest_dir.join(file_name);
            let mut out_file = File::create(&out_path).map_err(|e| {
                WadError::InvalidFantome(format!("failed to create output file: {e}"))
            })?;

            let mut buffer = [0u8; 64 * 1024];
            loop {
                let n = file.read(&mut buffer).map_err(|e| {
                    WadError::InvalidFantome(format!("error reading WAD entry: {e}"))
                })?;
                if n == 0 {
                    break;
                }
                out_file.write_all(&buffer[..n]).map_err(|e| {
                    WadError::InvalidFantome(format!("error writing WAD file: {e}"))
                })?;
            }

            extracted_wads.push(out_path);
        }
    }

    if extracted_wads.is_empty() {
        warn!(
            dest = %dest_dir.display(),
            entries = archive.len(),
            "Fantome package carried no WAD/ entries; nothing to inject"
        );
    } else {
        debug!(
            dest = %dest_dir.display(),
            wads = extracted_wads.len(),
            "Fantome WADs extracted"
        );
    }

    Ok(extracted_wads)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use zip::write::{SimpleFileOptions, ZipWriter};

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

    #[test]
    fn test_read_fantome_manifest_and_extract_wads() {
        let mut zip_bytes = Vec::new();
        {
            let mut writer = ZipWriter::new(Cursor::new(&mut zip_bytes));
            let options = SimpleFileOptions::default();

            writer.start_file("META/info.json", options).unwrap();
            let json = r#"{
                "Name": "Project Vayne",
                "Author": "Riot Games",
                "Version": "1.0.0",
                "Description": "Cybernetic hunter skin",
                "ChampId": 67
            }"#;
            writer.write_all(json.as_bytes()).unwrap();

            writer.start_file("WAD/Vayne.wad.client", options).unwrap();
            writer.write_all(b"RW\x03\x04fake_wad_content").unwrap();

            writer.finish().unwrap();
        }

        let manifest = read_fantome_manifest(Cursor::new(&zip_bytes)).expect("read manifest");
        assert_eq!(manifest.name, "Project Vayne");
        assert_eq!(manifest.author, "Riot Games");
        assert_eq!(manifest.champ_id, Some(67));

        let temp_dir = std::env::temp_dir().join("dekan_test_fantome_extract");
        let wads = extract_fantome_wads(Cursor::new(&zip_bytes), &temp_dir).expect("extract wads");
        assert_eq!(wads.len(), 1);
        assert_eq!(wads[0].file_name().unwrap(), "Vayne.wad.client");
        assert_eq!(
            std::fs::read(&wads[0]).unwrap(),
            b"RW\x03\x04fake_wad_content"
        );

        std::fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn test_rejects_missing_manifest() {
        let mut zip_bytes = Vec::new();
        {
            let mut writer = ZipWriter::new(Cursor::new(&mut zip_bytes));
            let options = SimpleFileOptions::default();
            writer.start_file("other.txt", options).unwrap();
            writer.write_all(b"not fantome").unwrap();
            writer.finish().unwrap();
        }

        let err = read_fantome_manifest(Cursor::new(&zip_bytes)).unwrap_err();
        assert!(matches!(err, WadError::InvalidFantome(_)));
    }
}
