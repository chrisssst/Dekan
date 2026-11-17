use crate::error::InjectError;
use crate::overlay_process::OverlayProcess;

impl super::InjectionPipeline {
    pub(super) async fn spawn_patcher(
        &self,
        mods: &[String],
    ) -> Result<OverlayProcess, InjectError> {
        match self.config.provider_kind {
            crate::provider::ProviderKind::Ltf => {
                let tools_dirs = dekan_platform::paths::tools_dir_candidates(
                    &self.config.overlay_config.overlay_dir,
                );
                crate::ltf::spawn_ltf_patcher(
                    &tools_dirs,
                    &self.config.overlay_config.mods_dir,
                    mods,
                    self.state_tx.clone(),
                )
                .await
            }
            crate::provider::ProviderKind::Native => {
                let host_exe = self.config.native_host_exe.as_deref().ok_or_else(|| {
                    InjectError::Process("Native host path not configured".into())
                })?;
                let dll_path =
                    self.config.native_dll_path.as_deref().ok_or_else(|| {
                        InjectError::Process("Native DLL path not configured".into())
                    })?;
                let target = self.config.native_target.as_ref().ok_or_else(|| {
                    InjectError::Process("Native game target not configured".into())
                })?;
                const DEFAULT_PIPE_NAME: &str = r"\\.\pipe\dekan-patcher-pipe";
                OverlayProcess::spawn_native_game_host(
                    host_exe,
                    dll_path,
                    &self.config.overlay_config.overlay_dir,
                    DEFAULT_PIPE_NAME,
                    &target.window_title,
                    &target.image_name,
                )
                .await
            }
            crate::provider::ProviderKind::Mock => Err(InjectError::Process(
                "Mock provider cannot spawn a patcher process".into(),
            )),
        }
    }
}
