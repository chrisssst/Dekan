use crate::error::InjectError;
use crate::pipeline::PipelineConfig;
use crate::provider::ProviderKind;

pub fn validate_patcher_binaries(config: &PipelineConfig) -> Result<(), InjectError> {
    if config.provider_kind != ProviderKind::Native {
        return Ok(());
    }
    let host_exe = config
        .native_host_exe
        .as_ref()
        .ok_or_else(|| InjectError::Process("Native host path not configured".into()))?;
    let dll_path = config
        .native_dll_path
        .as_ref()
        .ok_or_else(|| InjectError::Process("Native DLL path not configured".into()))?;
    for (what, path) in [("host", host_exe), ("DLL", dll_path)] {
        if !path.is_file() {
            return Err(InjectError::Process(format!(
                "Native {what} not found at '{}'",
                path.display()
            )));
        }
        crate::trust::verify_native_file(path)?;
    }
    Ok(())
}

impl super::InjectionPipeline {
    pub(super) fn require_game_integration(&self) -> Result<(), InjectError> {
        let (code, reason) = match self.config.provider_kind {
            crate::provider::ProviderKind::Native if self.config.native_target.is_some() => {
                return Ok(());
            }
            crate::provider::ProviderKind::Native => (
                "NATIVE_GAME_TARGET_MISSING",
                "the native engine needs the game window target before it can hook the game",
            ),
            crate::provider::ProviderKind::Mock => (
                "MOCK_PROVIDER_NOT_EXECUTABLE",
                "Mock providers cannot execute the production injection pipeline",
            ),
        };
        let message = format!("{code}: {reason}");
        tracing::warn!(provider = ?self.config.provider_kind, error_code = code, "Game integration unavailable before filesystem or process operations");
        self.publish(dekan_core::state::InjectionStatus::Failed {
            error: message.clone(),
        });
        Err(InjectError::Process(message))
    }
}
