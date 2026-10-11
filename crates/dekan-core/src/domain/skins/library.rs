use tracing::{debug, warn};

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryChroma {
    pub id: u32,
    pub package: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibrarySkin {
    pub id: u32,
    pub package: PathBuf,
    pub chromas: Vec<LibraryChroma>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChampionLibrary {
    pub champion_id: u32,
    pub skins: Vec<LibrarySkin>,
}

impl ChampionLibrary {
    #[must_use]
    pub fn skin(&self, skin_id: u32) -> Option<&LibrarySkin> {
        self.skins.iter().find(|s| s.id == skin_id)
    }

    #[must_use]
    pub fn package_for(&self, id: u32) -> Option<&Path> {
        for skin in &self.skins {
            if skin.id == id {
                return Some(&skin.package);
            }
            if let Some(chroma) = skin.chromas.iter().find(|c| c.id == id) {
                return Some(&chroma.package);
            }
        }
        None
    }

    #[must_use]
    pub fn package_count(&self) -> usize {
        self.skins
            .iter()
            .map(|s| 1 + s.chromas.len())
            .sum::<usize>()
    }
}

fn numeric_dir_name(entry: &std::fs::DirEntry) -> Option<u32> {
    if !entry.file_type().ok()?.is_dir() {
        return None;
    }
    entry.file_name().to_str()?.parse::<u32>().ok()
}

fn package_in(dir: &Path, id: u32) -> Option<PathBuf> {
    let candidate = dir.join(format!("{id}.fantome"));
    candidate.is_file().then_some(candidate)
}

#[must_use]
pub fn scan_champion(root: &Path, champion_id: u32) -> ChampionLibrary {
    let champion_dir = root.join(champion_id.to_string());
    let mut skins = Vec::new();

    let entries = match std::fs::read_dir(&champion_dir) {
        Ok(entries) => entries,

        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            debug!(
                champion_id,
                dir = %champion_dir.display(),
                "No library folder for this champion"
            );
            return ChampionLibrary { champion_id, skins };
        }
        Err(e) => {
            warn!(
                champion_id,
                dir = %champion_dir.display(),
                error = %e,
                "Library folder for this champion could not be read"
            );
            return ChampionLibrary { champion_id, skins };
        }
    };

    let mut skipped_not_numeric = 0usize;
    let mut skipped_no_package = 0usize;

    for entry in entries.flatten() {
        let Some(skin_id) = numeric_dir_name(&entry) else {
            skipped_not_numeric += 1;
            continue;
        };
        let skin_dir = entry.path();
        let Some(package) = package_in(&skin_dir, skin_id) else {
            skipped_no_package += 1;
            continue;
        };

        let mut chromas = Vec::new();
        if let Ok(inner) = std::fs::read_dir(&skin_dir) {
            for chroma_entry in inner.flatten() {
                let Some(chroma_id) = numeric_dir_name(&chroma_entry) else {
                    continue;
                };
                if let Some(chroma_package) = package_in(&chroma_entry.path(), chroma_id) {
                    chromas.push(LibraryChroma {
                        id: chroma_id,
                        package: chroma_package,
                    });
                }
            }
        }
        chromas.sort_by_key(|c| c.id);

        skins.push(LibrarySkin {
            id: skin_id,
            package,
            chromas,
        });
    }

    skins.sort_by_key(|s| s.id);

    if skipped_no_package > 0 || skipped_not_numeric > 0 {
        debug!(
            champion_id,
            skins = skins.len(),
            skipped_no_package,
            skipped_not_numeric,
            "Library folders skipped while indexing this champion"
        );
    }

    ChampionLibrary { champion_id, skins }
}

#[must_use]
pub fn champions_with_content(root: &Path) -> BTreeMap<u32, usize> {
    let mut found = BTreeMap::new();
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,

        Err(e) => {
            warn!(
                root = %root.display(),
                error = %e,
                "Skin library root could not be read; no skin will be offered"
            );
            return found;
        }
    };

    for entry in entries.flatten() {
        let Some(champion_id) = numeric_dir_name(&entry) else {
            continue;
        };
        let count = scan_champion(root, champion_id).package_count();
        if count > 0 {
            found.insert(champion_id, count);
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempdir::TempRoot, PathBuf) {
        let root = tempdir::TempRoot::new("dekan_library_test");
        let base = root.path().to_path_buf();

        let skin = base.join("238").join("238001");
        std::fs::create_dir_all(&skin).unwrap();
        std::fs::write(skin.join("238001.fantome"), b"skin").unwrap();
        for chroma in [238004u32, 238005] {
            let dir = skin.join(chroma.to_string());
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(format!("{chroma}.fantome")), b"chroma").unwrap();
        }

        let plain = base.join("238").join("238002");
        std::fs::create_dir_all(&plain).unwrap();
        std::fs::write(plain.join("238002.fantome"), b"skin").unwrap();

        std::fs::create_dir_all(base.join("238").join("238003")).unwrap();

        (root, base)
    }

    #[test]
    fn test_scans_skins_and_nested_chromas() {
        let (_guard, root) = fixture();
        let library = scan_champion(&root, 238);

        assert_eq!(library.skins.len(), 2, "empty folder must not be listed");
        assert_eq!(library.skins[0].id, 238001);
        assert_eq!(
            library.skins[0]
                .chromas
                .iter()
                .map(|c| c.id)
                .collect::<Vec<_>>(),
            vec![238004, 238005]
        );
        assert!(library.skins[1].chromas.is_empty());
        assert_eq!(library.package_count(), 4);
    }

    #[test]
    fn test_resolves_package_for_skin_and_chroma_alike() {
        let (_guard, root) = fixture();
        let library = scan_champion(&root, 238);

        assert!(
            library
                .package_for(238001)
                .is_some_and(|p| p.ends_with("238001.fantome"))
        );
        assert!(
            library
                .package_for(238005)
                .is_some_and(|p| p.ends_with("238005.fantome"))
        );
        assert!(library.package_for(999999).is_none());
    }

    #[test]
    fn test_missing_champion_is_an_empty_library_not_an_error() {
        let (_guard, root) = fixture();
        let library = scan_champion(&root, 1);
        assert!(library.skins.is_empty());
        assert_eq!(library.package_count(), 0);
    }

    #[test]
    fn test_missing_root_is_tolerated() {
        let library = scan_champion(Path::new(r"C:\definitely\not\here"), 238);
        assert!(library.skins.is_empty());
        assert!(champions_with_content(Path::new(r"C:\definitely\not\here")).is_empty());
    }

    #[test]
    fn test_champions_with_content_counts_packages() {
        let (_guard, root) = fixture();
        let found = champions_with_content(&root);
        assert_eq!(found.get(&238), Some(&4));
    }

    mod tempdir {
        use std::path::{Path, PathBuf};

        pub struct TempRoot(PathBuf);

        impl TempRoot {
            pub fn new(tag: &str) -> Self {
                static NEXT: std::sync::atomic::AtomicUsize =
                    std::sync::atomic::AtomicUsize::new(0);
                let unique = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let path =
                    std::env::temp_dir().join(format!("{tag}_{}_{unique}", std::process::id()));
                std::fs::create_dir_all(&path).unwrap();
                Self(path)
            }

            pub fn path(&self) -> &Path {
                &self.0
            }
        }

        impl Drop for TempRoot {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0); // ignore-ok: test fixture teardown
            }
        }
    }
}
