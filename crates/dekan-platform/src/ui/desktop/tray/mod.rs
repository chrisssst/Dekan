#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "macos")]
pub use macos::{SystemTray, TrayController};
#[cfg(windows)]
pub use windows::{SystemTray, TrayController};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayEvent {
    OpenLogs,

    OpenTools,

    OpenMods,

    RestoreMods,

    About,

    ToggleAutostart,

    ToggleAutoAccept,

    ToggleInjection,

    ToggleRandomSkin,
    ToggleLightLoading,

    Quit,

    Activated,

    PartyCreate,

    PartyJoin,

    PartyLeave,

    OpenRelease,
    InstallInjector,

    MarkProblem,

    ExportDiagnostics,
}
