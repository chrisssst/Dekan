use std::path::{Path, PathBuf};

#[derive(Debug, PartialEq, Eq)]
pub enum InjectorRefusal {
    Missing,
    NotAudited(Vec<(PathBuf, String)>),
}

#[must_use]
pub fn injector_refusal(
    host: &Path,
    host_hash: &str,
    dll: &Path,
    dll_hash: &str,
) -> Option<InjectorRefusal> {
    if !host.is_file() || !dll.is_file() {
        return Some(InjectorRefusal::Missing);
    }
    let refused: Vec<(PathBuf, String)> = [(host, host_hash), (dll, dll_hash)]
        .into_iter()
        .filter_map(|(file, hash)| {
            dekan_inject::dll_validator::validate_binary_hashes(file, &[hash])
                .err()
                .map(|e| (file.to_path_buf(), e.to_string()))
        })
        .collect();
    (!refused.is_empty()).then_some(InjectorRefusal::NotAudited(refused))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tools(name: &str, host: Option<&[u8]>, dll: Option<&[u8]>) -> (PathBuf, PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("dekan_startup_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir); // ignore-ok: fixture may not exist yet
        std::fs::create_dir_all(&dir).expect("fixture dir");
        let host_path = dir.join("ltk_patcher_host.exe");
        let dll_path = dir.join("ltk_patcher_dll.dll");
        if let Some(bytes) = host {
            std::fs::write(&host_path, bytes).expect("host");
        }
        if let Some(bytes) = dll {
            std::fs::write(&dll_path, bytes).expect("dll");
        }
        (dir, host_path, dll_path)
    }

    fn hash(bytes: &[u8]) -> String {
        dekan_inject::dll_validator::compute_sha256(bytes)
    }

    #[test]
    fn test_a_missing_file_refuses_the_start() {
        for (name, host, dll) in [
            ("none", None, None),
            ("no_dll", Some(&b"host"[..]), None),
            ("no_host", None, Some(&b"dll"[..])),
        ] {
            let (dir, host_path, dll_path) = tools(name, host, dll);
            assert_eq!(
                injector_refusal(&host_path, &hash(b"host"), &dll_path, &hash(b"dll")),
                Some(InjectorRefusal::Missing),
                "{name}"
            );
            let _ = std::fs::remove_dir_all(&dir); // ignore-ok: fixture cleanup
        }
    }

    #[test]
    fn test_files_that_are_not_the_audited_build_refuse_the_start() {
        let (dir, host_path, dll_path) = tools("bad", Some(b"host"), Some(b"patched dll"));
        match injector_refusal(&host_path, &hash(b"host"), &dll_path, &hash(b"dll")) {
            Some(InjectorRefusal::NotAudited(files)) => {
                assert_eq!(files.len(), 1, "only the DLL differs");
                assert_eq!(files[0].0, dll_path);
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir); // ignore-ok: fixture cleanup
    }

    #[test]
    fn test_the_audited_files_start() {
        let (dir, host_path, dll_path) = tools("good", Some(b"host"), Some(b"dll"));
        assert_eq!(
            injector_refusal(&host_path, &hash(b"host"), &dll_path, &hash(b"dll")),
            None
        );
        let _ = std::fs::remove_dir_all(&dir); // ignore-ok: fixture cleanup
    }
}
