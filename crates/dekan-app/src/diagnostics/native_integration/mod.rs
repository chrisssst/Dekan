pub mod event;
pub mod helpers;
pub mod probe;
#[cfg(test)]
mod tests;
pub use event::{
    DiagnosticEvent, DiagnosticOpStatus, DiagnosticSummary, DiagnosticTransition,
    IpcLatencyPercentiles,
};
use std::path::Path;

pub async fn run_native_diagnostics(
    tools_dir: &Path,
    report_path: &Path,
) -> Result<DiagnosticSummary, String> {
    let summary = probe::execute_probe(tools_dir).await;
    let json = serde_json::to_vec_pretty(&summary)
        .map_err(|e| format!("Diagnostic serialization failed: {e}"))?;
    std::fs::write(report_path, json).map_err(|e| {
        format!(
            "Diagnostic report write failed at {}: {e}",
            report_path.display()
        )
    })?;
    tracing::info!(correlation_id = %summary.correlation_id, verdict = %summary.verdict, report = %report_path.display(), "Native diagnostic snapshot saved; runtime consumption and rendering not tested");
    Ok(summary)
}
