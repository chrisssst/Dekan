use super::*;

#[derive(Debug, Clone)]
pub struct ResolvedPaths {
    pub tools_dir: PathBuf,
    pub tools_source: ToolsSource,
    pub ltk_host_exe: PathBuf,
    pub ltk_dll_path: PathBuf,
    pub library_dir: PathBuf,
    pub mods_dir: PathBuf,
    pub overlay_dir: PathBuf,
    pub state_dir: PathBuf,
    pub game_dir: PathBuf,

    pub mod_roots: Vec<dekan_core::mods::ModRoot>,

    pub custom_mods_root: PathBuf,
}

impl ResolvedPaths {
    pub fn discover() -> Self {
        let app_state_dir =
            state_dir().unwrap_or_else(|_| std::env::temp_dir().join("Dekan_state"));
        let app_data_dir = data_dir().unwrap_or_else(|_| std::env::temp_dir().join("Dekan"));
        Self::discover_in(app_state_dir, app_data_dir)
    }

    pub fn discover_in(app_state_dir: PathBuf, app_data_dir: PathBuf) -> Self {
        let candidate_tools_dirs = tools_dir_candidates(&app_data_dir);
        let (tools_dir, source) = resolve_tools_dir(&candidate_tools_dirs);

        match source {
            ToolsSource::Own => {
                info!(tools = %tools_dir.display(), "Injection tools found in Dekan's own folder");
            }
            ToolsSource::Missing => {
                error!(
                    expected = %tools_dir.display(),
                    "Injection tools not found. Place ltk_patcher_host.exe and ltk_patcher_dll.dll \
                     in that folder — no skin can be injected until then"
                );
            }
        }

        let ltk_host_exe = tools_dir.join(dekan_inject::ltk_host::HOST_EXE);
        let ltk_dll_path = tools_dir.join(dekan_inject::ltk_host::DLL_FILE);

        let mut candidate_library_dirs =
            vec![app_data_dir.join("library"), app_data_dir.join("skins")];
        if let Some(tool_lib) = tools_dir.parent().map(|p| p.join("library")) {
            candidate_library_dirs.push(tool_lib);
        }

        let library_dir = candidate_library_dirs
            .iter()
            .find(|p| p.is_dir())
            .cloned()
            .unwrap_or_else(|| candidate_library_dirs[0].clone());

        let game_dir = match dekan_platform::paths::discover_game_dir() {
            Some(dir) => dir,
            None => {
                warn!(
                    "League install not found yet (client closed and no install registered by the \
                     Riot Client); it will be looked up again when a skin is prepared"
                );
                PathBuf::new()
            }
        };

        let overlay_dir = app_data_dir.join("overlay");
        let mods_dir = app_data_dir.join("mods");
        let custom_mods_root = dekan_app::mods_store::dekan_mods_root(&app_data_dir);
        let mod_roots = dekan_app::mods_store::mod_roots(&app_data_dir);

        Self {
            tools_dir,
            tools_source: source,
            ltk_host_exe,
            ltk_dll_path,
            library_dir,
            mods_dir,
            overlay_dir,
            state_dir: app_state_dir,
            game_dir,
            mod_roots,
            custom_mods_root,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolsSource {
    Own,

    Missing,
}

pub(super) fn resolve_tools_dir(candidates: &[PathBuf]) -> (PathBuf, ToolsSource) {
    let complete = |dir: &PathBuf| {
        dir.join(dekan_inject::ltk_host::HOST_EXE).is_file()
            && dir.join(dekan_inject::ltk_host::DLL_FILE).is_file()
    };

    if let Some(dir) = candidates.iter().find(|dir| complete(dir)) {
        info!(tools = %dir.display(), "Injection backend (LTK host + DLL) found");
        return (dir.clone(), ToolsSource::Own);
    }

    (
        candidates.first().cloned().unwrap_or_default(),
        ToolsSource::Missing,
    )
}

pub fn required_tool_files(paths: &ResolvedPaths) -> Vec<PathBuf> {
    vec![paths.ltk_host_exe.clone(), paths.ltk_dll_path.clone()]
}

pub fn tools_ready(paths: &ResolvedPaths) -> bool {
    required_tool_files(paths).iter().all(|file| file.is_file())
}
