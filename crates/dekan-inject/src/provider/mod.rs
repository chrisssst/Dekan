use std::path::Path;
use tracing::info;

use dekan_core::state::InjectionStatus;

use crate::error::InjectError;
use crate::pipeline::{InjectionOutcome, PipelineConfig};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum ProviderKind {
    #[default]
    Ltf,
    Native,
    Mock,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderStatus {
    Ready,
    LaboratoryOnly { capabilities: Vec<&'static str> },
    Untrusted { reason: String },
    Unsupported { reason: String },
}

pub trait InjectorProvider: Send + Sync {
    fn name(&self) -> &'static str;

    fn kind(&self) -> ProviderKind;

    fn check_compatibility(&self, host_path: &Path, dll_path: &Path) -> ProviderStatus;

    fn execute(
        &self,
        config: &PipelineConfig,
        mods: &[String],
        game_pid: u32,
    ) -> impl std::future::Future<Output = Result<InjectionOutcome, InjectError>> + Send;
}

#[derive(Debug, Default)]
pub struct DekanNativeProvider;

impl DekanNativeProvider {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl InjectorProvider for DekanNativeProvider {
    fn name(&self) -> &'static str {
        "dekan-native"
    }

    fn kind(&self) -> ProviderKind {
        ProviderKind::Native
    }

    fn check_compatibility(&self, host_path: &Path, dll_path: &Path) -> ProviderStatus {
        if !host_path.exists() {
            return ProviderStatus::Unsupported {
                reason: format!("Native host binary not found at {}", host_path.display()),
            };
        }
        if !dll_path.exists() {
            return ProviderStatus::Unsupported {
                reason: format!("Native DLL binary not found at {}", dll_path.display()),
            };
        }
        ProviderStatus::LaboratoryOnly {
            capabilities: vec![
                "file_redirection",
                "virtual_overlay",
                "ipc_telemetry",
                "native_host",
                "custom_skins",
            ],
        }
    }

    async fn execute(
        &self,
        config: &PipelineConfig,
        mods: &[String],
        game_pid: u32,
    ) -> Result<InjectionOutcome, InjectError> {
        info!(
            provider = self.name(),
            game_pid, "Executing Dekan Native Provider orchestration request"
        );

        let mut native_config = config.clone();
        native_config.provider_kind = ProviderKind::Native;
        let pipeline = crate::pipeline::InjectionPipeline::new(native_config, None);
        pipeline.execute(mods, game_pid).await
    }
}

#[derive(Debug, Default)]
pub struct LtfProvider;

impl LtfProvider {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl InjectorProvider for LtfProvider {
    fn name(&self) -> &'static str {
        "ltf-bridge"
    }

    fn kind(&self) -> ProviderKind {
        ProviderKind::Ltf
    }

    fn check_compatibility(&self, _host_path: &Path, _dll_path: &Path) -> ProviderStatus {
        if crate::ltf::find_manager_manager().is_none() {
            return ProviderStatus::Unsupported {
                reason: "Mod Manager is not installed on this system".into(),
            };
        }
        ProviderStatus::Ready
    }

    async fn execute(
        &self,
        config: &PipelineConfig,
        mods: &[String],
        game_pid: u32,
    ) -> Result<InjectionOutcome, InjectError> {
        info!(
            provider = self.name(),
            game_pid, "Executing LTF Bridge Provider orchestration request"
        );

        let mut ltf_config = config.clone();
        ltf_config.provider_kind = ProviderKind::Ltf;
        let pipeline = crate::pipeline::InjectionPipeline::new(ltf_config, None);
        pipeline.execute(mods, game_pid).await
    }
}

#[derive(Debug, Default)]
pub struct MockProvider {
    pub status_to_return: Option<InjectionStatus>,
}

impl InjectorProvider for MockProvider {
    fn name(&self) -> &'static str {
        "mock"
    }

    fn kind(&self) -> ProviderKind {
        ProviderKind::Mock
    }

    fn check_compatibility(&self, _host_path: &Path, _dll_path: &Path) -> ProviderStatus {
        ProviderStatus::Ready
    }

    async fn execute(
        &self,
        _config: &PipelineConfig,
        _mods: &[String],
        _game_pid: u32,
    ) -> Result<InjectionOutcome, InjectError> {
        Ok(InjectionOutcome {
            status: self
                .status_to_return
                .clone()
                .unwrap_or(InjectionStatus::Confirmed),
            overlay: None,
        })
    }
}

#[cfg(test)]
mod tests;
