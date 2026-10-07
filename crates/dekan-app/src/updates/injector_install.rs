use std::fmt;
use std::path::{Path, PathBuf};

use crate::ltk_release::{INJECTOR_FILES, download_injector};

pub const INSTALL_FLAG: &str = "--install-injector";

const STAGING_DIR: &str = "injector_staging";

#[derive(Debug, PartialEq, Eq)]
pub enum InstallError {
    Denied,
    NotTrusted(&'static str, String),
    Failed(String),
}

impl fmt::Display for InstallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Denied => write!(f, "Windows denied writing to the tools folder"),
            Self::NotTrusted(name, why) => write!(f, "{name} is not trusted: {why}"),
            Self::Failed(why) => write!(f, "{why}"),
        }
    }
}

fn io_error(context: &str, e: &std::io::Error) -> InstallError {
    if e.kind() == std::io::ErrorKind::PermissionDenied {
        InstallError::Denied
    } else {
        InstallError::Failed(format!("{context}: {e}"))
    }
}

fn verify(name: &'static str, path: &Path) -> Result<(), InstallError> {
    dekan_inject::trust::verify_injector_file(path)
        .map_err(|e| InstallError::NotTrusted(name, e.to_string()))
}

pub async fn stage(version: &str, state_dir: &Path) -> Result<PathBuf, InstallError> {
    let files = download_injector(version)
        .await
        .map_err(InstallError::Failed)?;
    let dir = state_dir.join(STAGING_DIR);
    std::fs::create_dir_all(&dir).map_err(|e| io_error("creating the staging folder", &e))?;
    for (name, bytes) in files {
        let path = dir.join(name);
        std::fs::write(&path, bytes).map_err(|e| io_error("staging a download", &e))?;
        verify(name, &path)?;
    }
    Ok(dir)
}

pub fn install(staging: &Path, tools: &Path) -> Result<(), InstallError> {
    let mut files = Vec::with_capacity(INJECTOR_FILES.len());
    for name in INJECTOR_FILES {
        let bytes =
            std::fs::read(staging.join(name)).map_err(|e| io_error("reading a staged file", &e))?;
        files.push((name, bytes));
    }
    std::fs::create_dir_all(tools).map_err(|e| io_error("creating the tools folder", &e))?;
    let mut partials = Vec::with_capacity(files.len());
    let staged = files.into_iter().try_for_each(|(name, bytes)| {
        let partial = tools.join(format!("{name}.partial"));
        std::fs::write(&partial, &bytes).map_err(|e| io_error("writing the tools folder", &e))?;
        partials.push((name, partial.clone()));
        verify(name, &partial)
    });
    let placed = staged.and_then(|()| {
        partials.iter().try_for_each(|(name, partial)| {
            std::fs::rename(partial, tools.join(name)).map_err(|e| io_error("replacing a tool", &e))
        })
    });
    if placed.is_err() {
        for (_, partial) in &partials {
            let _ = std::fs::remove_file(partial); // ignore-ok: the install error is what gets reported
        }
    }
    placed
}

#[must_use]
pub fn elevated_parameters(staging: &Path, tools: &Path) -> String {
    format!(
        "{INSTALL_FLAG} \"{}\" \"{}\"",
        staging.display(),
        tools.display()
    )
}

#[must_use]
pub fn is_dekan_tools_folder(
    tools: &Path,
    install_dir: Option<&Path>,
    exe: Option<&Path>,
) -> bool {
    [install_dir, exe.and_then(Path::parent)]
        .into_iter()
        .flatten()
        .any(|dir| dir.join("tools") == tools)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("dekan_injector_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir); // ignore-ok: fixture may not exist yet
        std::fs::create_dir_all(dir.join("staging")).expect("staging");
        dir
    }

    #[test]
    fn unsigned_staged_files_are_refused_and_leave_nothing_in_the_tools_folder() {
        let root = fixture("unsigned");
        let staging = root.join("staging");
        std::fs::write(staging.join(INJECTOR_FILES[0]), b"host").expect("host");
        std::fs::write(staging.join(INJECTOR_FILES[1]), b"dll").expect("dll");
        let tools = root.join("tools");
        std::fs::create_dir_all(&tools).expect("tools");
        std::fs::write(tools.join(INJECTOR_FILES[1]), b"old dll").expect("old");

        assert!(matches!(
            install(&staging, &tools),
            Err(InstallError::NotTrusted(name, _)) if name == INJECTOR_FILES[0]
        ));
        let left: Vec<String> = std::fs::read_dir(&tools)
            .expect("list")
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            left,
            vec![INJECTOR_FILES[1].to_owned()],
            "the old file stays and no partial is left"
        );
        assert_eq!(
            std::fs::read(tools.join(INJECTOR_FILES[1])).expect("old"),
            b"old dll"
        );
        let _ = std::fs::remove_dir_all(&root); // ignore-ok: fixture cleanup
    }

    #[test]
    #[ignore = "needs the LTK injector in Dekan's tools folder"]
    fn signed_files_replace_the_tools_and_leave_no_partial_behind() {
        let installed = Path::new(r"C:\Program Files\Dekan\tools");
        let root = fixture("signed");
        let staging = root.join("staging");
        for name in INJECTOR_FILES {
            std::fs::copy(installed.join(name), staging.join(name)).expect("copy");
        }
        let tools = root.join("tools");
        install(&staging, &tools).expect("install");
        assert_eq!(std::fs::read_dir(&tools).expect("list").count(), 2);
        let _ = std::fs::remove_dir_all(&root); // ignore-ok: fixture cleanup
    }

    #[test]
    fn only_dekans_own_tools_folders_are_accepted_as_a_target() {
        let install_dir = Path::new(r"C:\Program Files\Dekan");
        let exe = Path::new(r"D:\Portable\Dekan\dekan.exe");
        let accepts =
            |tools: &str| is_dekan_tools_folder(Path::new(tools), Some(install_dir), Some(exe));
        assert!(accepts(r"C:\Program Files\Dekan\tools"));
        assert!(accepts(r"D:\Portable\Dekan\tools"));
        assert!(!accepts(r"C:\Windows\System32"));
        assert!(!accepts(r"C:\Program Files\Dekan"));
    }

    #[test]
    fn the_elevated_command_line_quotes_both_paths() {
        assert_eq!(
            elevated_parameters(
                Path::new(r"C:\Users\A B\staging"),
                Path::new(r"C:\Program Files\Dekan\tools")
            ),
            r#"--install-injector "C:\Users\A B\staging" "C:\Program Files\Dekan\tools""#
        );
    }
}
