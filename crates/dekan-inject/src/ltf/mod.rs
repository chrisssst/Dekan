pub mod client;
pub mod discovery;
pub mod launcher;
pub mod log_tail;
pub mod native_bundle;
pub mod session;

pub use client::BridgeClient;
pub use discovery::{BridgeEndpoint, discover_bridge};
pub use launcher::{
    MIN_MANAGER_VERSION, ManagerInstallState, check_manager_install, ensure_bridge_running,
    find_launcher_tools, find_manager_manager, wait_for_bridge,
};
pub use session::{LtfSession, pack_mod_fantome, spawn_ltf_patcher, warmup_bridge};

pub async fn shutdown_bridge() {
    if let Some(endpoint) = discover_bridge(None).await {
        if let Err(error) = BridgeClient::new(endpoint).kill().await {
            tracing::warn!(%error, "Could not shut down the bridge");
        }
    }
}
