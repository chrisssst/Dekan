use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::Result;
use dekan_core::{
    overlay::{Catalog, OverlayCommand},
    phase::GamePhase,
};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

mod cdp;
mod script;
mod shim;
#[cfg(test)]
mod tests;

pub(crate) struct Frame<'a> {
    pub enabled: bool,
    pub use_native: bool,
    pub phase: GamePhase,
    pub tools: &'a [PathBuf],
    pub catalog: Option<&'a Catalog>,
    pub selected: Option<u32>,
}

#[derive(Default)]
pub(crate) struct NativeClient {
    requested: bool,
    initialized: bool,
    attempted: bool,
    owned: bool,
    restart: bool,
    launcher: Option<PathBuf>,
    helper: Option<(bool, JoinHandle<Result<bool>>)>,
    connection: Option<cdp::Connection>,
    endpoint: Option<(u32, u16)>,
    retry: Option<Instant>,
    last_error: Option<String>,
}

impl NativeClient {
    pub(crate) async fn sync(&mut self, frame: Frame<'_>) -> (Vec<OverlayCommand>, bool) {
        self.finish_helper().await;
        if !self.initialized {
            self.initialized = true;
            if let Ok(launcher) = shim::launcher(frame.tools) {
                self.owned = shim::registration(&launcher) == shim::Registration::Ours;
                self.launcher = Some(launcher);
            }
        }

        if frame.enabled != self.requested {
            self.requested = frame.enabled;
            self.attempted = false;
            self.retry = None;
            self.last_error = None;
            info!(enabled = frame.enabled, "Native client mode requested");
        }
        if !frame.enabled {
            self.restart = false;
            self.disconnect().await;
            if self.owned && self.helper.is_none() && !self.attempted {
                self.start_helper(false);
            }
            return (Vec::new(), false);
        }
        if self.helper.is_some() || self.retry.is_some_and(|deadline| Instant::now() < deadline) {
            return (Vec::new(), false);
        }
        let endpoint = shim::client_endpoint();
        let safe_restart = matches!(frame.phase, GamePhase::None | GamePhase::Lobby);
        if endpoint.is_none() && !self.attempted && safe_restart {
            match shim::launcher(frame.tools) {
                Ok(launcher) => {
                    self.launcher = Some(launcher);
                    self.start_helper(true);
                }
                Err(e) => {
                    self.attempted = true;
                    self.problem(&e);
                }
            }
            return (Vec::new(), false);
        }
        if self.restart && endpoint.is_none() && safe_restart {
            self.retry = Some(Instant::now() + Duration::from_secs(3));
            match dekan_lcu::client::LcuClient::discover().await {
                Ok(client) => match client.restart_ux().await {
                    Ok(true) => {
                        self.restart = false;
                        info!("Restarted League client interface for native mode");
                    }
                    Ok(false) => self.problem(&anyhow::anyhow!(
                        "League client refused the interface restart"
                    )),
                    Err(e) => self.problem(&e.into()),
                },
                Err(e) => self.problem(&e.into()),
            }
            return (Vec::new(), false);
        }
        if !frame.use_native || frame.phase == GamePhase::InProgress {
            self.disconnect().await;
            return (Vec::new(), false);
        }
        let Some(endpoint) = endpoint else {
            return (Vec::new(), false);
        };
        if self.endpoint != Some(endpoint) {
            self.disconnect().await;
        }
        let outcome = self.exchange(endpoint, frame.catalog, frame.selected).await;
        match outcome {
            Ok(value) => {
                self.last_error = None;
                let active = frame.catalog.is_some_and(|c| c.champion_id != 0)
                    && value["active"].as_bool() == Some(true);
                (script::commands(&value["commands"]), active)
            }
            Err(e) => {
                self.problem(&e);
                self.connection = None;
                self.endpoint = None;
                self.retry = Some(Instant::now() + Duration::from_secs(2));
                (Vec::new(), false)
            }
        }
    }

    async fn exchange(
        &mut self,
        endpoint: (u32, u16),
        catalog: Option<&Catalog>,
        selected: Option<u32>,
    ) -> Result<serde_json::Value> {
        if self.connection.is_none() {
            let mut connection = cdp::Connection::discover(endpoint.1).await?;
            if connection.evaluate(&script::install()?).await? != true {
                anyhow::bail!("Dekan client script did not initialize");
            }
            self.endpoint = Some(endpoint);
            self.connection = Some(connection);
            info!(
                pid = endpoint.0,
                "Connected native Dekan interface to League client"
            );
        }
        let language = match dekan_platform::i18n::active_language() {
            dekan_platform::i18n::Language::Turkish => "tr",
            dekan_platform::i18n::Language::English => "en",
        };
        let expression = script::sync(&script::snapshot(catalog), selected, language);
        let connection = self
            .connection
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("Native connection missing"))?;
        let mut value = connection.evaluate(&expression).await?;
        if value.is_null() {
            connection.evaluate(&script::install()?).await?;
            value = connection.evaluate(&expression).await?;
        }
        anyhow::ensure!(
            value.is_object(),
            "Native interface returned an invalid response"
        );
        Ok(value)
    }

    fn start_helper(&mut self, install: bool) {
        self.attempted = true;
        if let Some(launcher) = self.launcher.clone() {
            self.helper = Some((
                install,
                tokio::task::spawn_blocking(move || shim::change(&launcher, install)),
            ));
        }
    }

    async fn finish_helper(&mut self) {
        if !self
            .helper
            .as_ref()
            .is_some_and(|(_, task)| task.is_finished())
        {
            return;
        }
        if let Some((install, task)) = self.helper.take() {
            match task.await {
                Ok(Ok(true)) => {
                    self.owned = install;
                    self.restart = install;
                    self.retry = None;
                    info!(
                        installed = install,
                        "Native client helper registration updated"
                    );
                }
                Ok(Ok(false)) => warn!(
                    install,
                    "Native client helper was declined or did not complete; keeping the Dekan window"
                ),
                Ok(Err(e)) => self.problem(&e),
                Err(e) => self.problem(&e.into()),
            }
        }
    }

    fn problem(&mut self, error: &anyhow::Error) {
        let reason = error.to_string();
        if self.last_error.as_ref() != Some(&reason) {
            warn!(error = %reason, "Native client mode unavailable; keeping the Dekan window");
            self.last_error = Some(reason);
        }
    }

    async fn disconnect(&mut self) {
        if let Some(mut connection) = self.connection.take() {
            if let Err(e) = connection.evaluate(script::CLEANUP).await {
                debug!(error = %e, "Native client interface already disconnected during cleanup");
            }
        }
        self.endpoint = None;
    }

    pub(crate) async fn shutdown(&mut self) {
        self.disconnect().await;
        if let Some((install, task)) = self.helper.take() {
            match task.await {
                Ok(Ok(true)) => self.owned = install,
                Ok(Ok(false)) => {}
                Ok(Err(e)) => self.problem(&e),
                Err(e) => self.problem(&e.into()),
            }
        }
        if self.owned {
            if let Some(launcher) = self.launcher.take() {
                match tokio::task::spawn_blocking(move || shim::change(&launcher, false)).await {
                    Ok(Ok(true)) => self.owned = false,
                    Ok(Ok(false)) => warn!(
                        "Native client helper removal declined; disable native mode on next start to retry"
                    ),
                    Ok(Err(e)) => self.problem(&e),
                    Err(e) => self.problem(&e.into()),
                }
            }
        }
    }
}
