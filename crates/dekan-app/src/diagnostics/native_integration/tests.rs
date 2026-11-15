use super::event::{DiagnosticOpStatus, DiagnosticTransition};
use super::probe::execute_probe;

#[tokio::test]
async fn missing_artifacts_cannot_generate_runtime_success_evidence() {
    let temp = std::env::temp_dir().join(format!("dekan_diag_absent_{}", uuid::Uuid::new_v4()));
    let summary = execute_probe(&temp).await;
    assert_eq!(summary.verdict, "NATIVE_GAME_RUNTIME_NOT_TESTED");
    assert!(!summary.laboratory_engine_ready);
    assert!(summary.target_champion.is_none());
    assert!(summary.target_skin.is_none());
    assert_eq!(summary.total_asset_count, 0);
    assert!(summary.ipc_latency_percentiles.is_none());
    assert!(summary.events.iter().any(|event| matches!(
        event.transition,
        Some(DiagnosticTransition::IntegrationRuntimeNotTested)
    )));
    assert!(
        !serde_json::to_string(&summary)
            .expect("serialize")
            .contains("NATIVE_GAME_TARGET_MISSING")
    );
    for event in &summary.events {
        if matches!(
            event.transition,
            Some(
                DiagnosticTransition::NativeHostStarted
                    | DiagnosticTransition::NativeDllLoaded
                    | DiagnosticTransition::ResourceConsumed
                    | DiagnosticTransition::OverlayBuildCompleted
                    | DiagnosticTransition::IpcConnected
            )
        ) {
            assert_ne!(event.operation_status, DiagnosticOpStatus::Completed);
        }
    }
    assert!(!temp.exists());
}

#[tokio::test]
async fn diagnostic_report_write_failure_is_reported() {
    let temp = std::env::temp_dir().join(format!("dekan_diag_write_{}", uuid::Uuid::new_v4()));
    let result = super::run_native_diagnostics(&temp, &temp.join("absent/report.json")).await;
    assert!(result.is_err());
    assert!(!temp.exists());
}

#[test]
fn test_diagnostic_op_status_serialization() {
    let status = DiagnosticOpStatus::NotImplemented;
    let json = serde_json::to_string(&status).expect("serialize");
    assert_eq!(json, "\"NOT_IMPLEMENTED\"");

    let status = DiagnosticOpStatus::Completed;
    let json = serde_json::to_string(&status).expect("serialize");
    assert_eq!(json, "\"COMPLETED\"");
}
