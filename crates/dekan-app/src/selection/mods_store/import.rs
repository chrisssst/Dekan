use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportRefusal {
    UnsupportedExtension,
    NotAModPackage(String),
    NoManifest,
    NoContent,

    NoChampion,
    Io(String),
}

impl std::fmt::Display for ImportRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedExtension => {
                write!(f, "only .fantome, .zip and .modpkg mods can be imported")
            }
            Self::NotAModPackage(e) => write!(f, "not a mod package: {e}"),
            Self::NoManifest => write!(f, "missing META/info.json manifest"),
            Self::NoContent => write!(f, "the package has no WAD/ or RAW/ content"),
            Self::NoChampion => write!(
                f,
                "the package does not say which champion it is for and no champion is selected"
            ),
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}

impl ImportRefusal {
    #[must_use]
    pub fn describe(&self, text: &dekan_platform::i18n::Text) -> String {
        use dekan_platform::i18n::fill;
        match self {
            Self::UnsupportedExtension => text.import_unsupported_extension.to_owned(),
            Self::NotAModPackage(e) => fill(text.import_not_a_mod, "error", e),
            Self::NoManifest => text.import_no_manifest.to_owned(),
            Self::NoContent => text.import_no_content.to_owned(),
            Self::NoChampion => text.import_no_champion.to_owned(),
            Self::Io(e) => fill(text.import_io_error, "error", e),
        }
    }
}

pub(super) const IMPORT_NAME_MAX: usize = 60;

pub fn import_archive(
    own_root: &Path,
    category: ModCategory,
    champion_id: Option<ChampionId>,
    source: &Path,
) -> Result<PathBuf, ImportRefusal> {
    let extension = source
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .filter(|e| e == "fantome" || e == "zip" || e == "modpkg")
        .ok_or(ImportRefusal::UnsupportedExtension)?;

    let open = || {
        std::fs::File::open(source)
            .map_err(|e| ImportRefusal::Io(format!("cannot open the file: {e}")))
    };
    let modpkg = extension == "modpkg";
    if modpkg {
        let package =
            read_modpkg(source).map_err(|e| ImportRefusal::NotAModPackage(e.to_string()))?;
        if package.wad_names().is_empty() {
            return Err(ImportRefusal::NoContent);
        }
    } else {
        let shape = dekan_wad::fantome::mod_archive_shape(open()?)
            .map_err(|e| ImportRefusal::NotAModPackage(e.to_string()))?;
        if !shape.manifest {
            return Err(ImportRefusal::NoManifest);
        }
        if !shape.content {
            return Err(ImportRefusal::NoContent);
        }
    }

    let mut dir = own_root.join(category.folder());
    if category == ModCategory::Skin {
        let names_a_champion = modpkg
            || dekan_wad::fantome::wad_names_in_archive(std::io::BufReader::new(open()?))
                .map(|names| !names.is_empty())
                .unwrap_or(false);
        if !names_a_champion {
            let champion = champion_id.ok_or(ImportRefusal::NoChampion)?;
            dir = dir.join(champion.to_string());
        }
    }
    std::fs::create_dir_all(&dir)
        .map_err(|e| ImportRefusal::Io(format!("cannot create {}: {e}", dir.display())))?;

    let stem: String = source
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || " _-().".contains(c) {
                c
            } else {
                '_'
            }
        })
        .take(IMPORT_NAME_MAX)
        .collect();
    let stem = stem.trim_matches(|c: char| c == ' ' || c == '.').to_owned();
    let stem = if stem.is_empty() {
        "mod".to_owned()
    } else {
        stem
    };
    let destination = (1..)
        .map(|n| {
            let name = if n == 1 {
                format!("{stem}.{extension}")
            } else {
                format!("{stem} ({n}).{extension}")
            };
            dir.join(name)
        })
        .find(|path| !path.exists())
        .ok_or_else(|| ImportRefusal::Io("no free file name".into()))?;

    let partial = destination.with_extension(format!("{extension}.partial"));
    std::fs::copy(source, &partial).map_err(|e| {
        let _ = std::fs::remove_file(&partial); // ignore-ok: best-effort cleanup of the half-copy; the copy error is what gets reported
        ImportRefusal::Io(format!("copy failed: {e}"))
    })?;
    std::fs::rename(&partial, &destination).map_err(|e| {
        let _ = std::fs::remove_file(&partial); // ignore-ok: best-effort cleanup; the rename error is what gets reported
        ImportRefusal::Io(format!("could not move the copy into place: {e}"))
    })?;
    Ok(destination)
}
