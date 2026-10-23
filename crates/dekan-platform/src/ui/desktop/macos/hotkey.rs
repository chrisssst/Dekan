use tokio::sync::mpsc::UnboundedSender;
use tracing::info;

use crate::tray::TrayEvent;

#[derive(Debug, Clone, Copy)]
pub struct MarkProblemHotkey(());

impl MarkProblemHotkey {
    pub fn arm(&self) {}

    pub fn disarm(&self) {}
}

#[must_use]
pub fn spawn_mark_problem_hotkey(_events: UnboundedSender<TrayEvent>) -> Option<MarkProblemHotkey> {
    info!("The mark-a-problem shortcut is not available on macOS; use the control panel button");
    None
}
