use std::path::{Path, PathBuf};

#[derive(Debug, PartialEq, Eq)]
pub enum InjectorRefusal {
    Missing,
    NotTrusted(Vec<(PathBuf, String)>),
}

#[must_use]
pub fn injector_refusal(host: &Path, dll: &Path) -> Option<InjectorRefusal> {
    if !host.is_file() || !dll.is_file() {
        return Some(InjectorRefusal::Missing);
    }
    let refused: Vec<(PathBuf, String)> = [host, dll]
        .into_iter()
        .filter_map(|file| {
            dekan_inject::trust::verify_injector_file(file)
                .err()
                .map(|e| (file.to_path_buf(), e.to_string()))
        })
        .collect();
    (!refused.is_empty()).then_some(InjectorRefusal::NotTrusted(refused))
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

    #[test]
    fn test_a_missing_file_refuses_the_start() {
        for (name, host, dll) in [
            ("none", None, None),
            ("no_dll", Some(&b"host"[..]), None),
            ("no_host", None, Some(&b"dll"[..])),
        ] {
            let (dir, host_path, dll_path) = tools(name, host, dll);
            assert_eq!(
                injector_refusal(&host_path, &dll_path),
                Some(InjectorRefusal::Missing),
                "{name}"
            );
            let _ = std::fs::remove_dir_all(&dir); // ignore-ok: fixture cleanup
        }
    }

    #[test]
    fn test_unsigned_files_refuse_the_start_and_both_are_named() {
        let (dir, host_path, dll_path) = tools("unsigned", Some(b"host"), Some(b"patched dll"));
        match injector_refusal(&host_path, &dll_path) {
            Some(InjectorRefusal::NotTrusted(files)) => {
                let named: Vec<&PathBuf> = files.iter().map(|(file, _)| file).collect();
                assert_eq!(named, vec![&host_path, &dll_path]);
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir); // ignore-ok: fixture cleanup
    }

    #[test]
    #[ignore = "needs the LTK injector in Dekan's tools folder"]
    fn test_the_publishers_signed_files_start() {
        let tools = Path::new(r"C:\Program Files\Dekan\tools");
        assert_eq!(
            injector_refusal(
                &tools.join("ltk_patcher_host.exe"),
                &tools.join("ltk_patcher_dll.dll")
            ),
            None
        );
    }
}
