use super::event::{DiagnosticEvent, DiagnosticOpStatus, DiagnosticSummary, DiagnosticTransition};
use super::helpers::{query_process_memory_usage, utc_now_micros};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::time::Instant;

pub async fn execute_probe(tools_dir: &Path) -> DiagnosticSummary {
    let started = Instant::now();
    let correlation_id = uuid::Uuid::new_v4().to_string();
    let selected = "dekan-native";
    let mut events = vec![DiagnosticEvent::new("configuration", "provider_selected", &correlation_id, selected, DiagnosticOpStatus::Completed, utc_now_micros())
        .with_transition(DiagnosticTransition::ProviderSelected).with_details(serde_json::json!({
            "scope": "preference_snapshot", "diagnostic_provider": "dekan-native", "owned_pid": std::process::id(), "pipeline_executed": false
        }))];
    let native_host = tools_dir.join("bullet-br.exe");
    let native_dll = tools_dir.join("bullet-br.dll");
    let mut available = true;
    for path in [&native_host, &native_dll] {
        let read_started = Instant::now();
        let (status, details) = match std::fs::read(path) {
            Ok(bytes) => (
                DiagnosticOpStatus::Completed,
                serde_json::json!({
                "path": path, "size_bytes": bytes.len(), "sha256": Sha256::digest(&bytes).iter().map(|byte| format!("{byte:02x}")).collect::<String>(), "loaded": false, "authenticode_verified": false
                }),
            ),
            Err(error) => {
                available = false;
                (
                    DiagnosticOpStatus::Failed,
                    serde_json::json!({"path": path, "error": error.to_string(), "os_error": error.raw_os_error()}),
                )
            }
        };
        events.push(
            DiagnosticEvent::new(
                "artifact",
                "artifact_read",
                &correlation_id,
                "dekan-native",
                status,
                utc_now_micros(),
            )
            .with_duration(
                read_started
                    .elapsed()
                    .as_millis()
                    .try_into()
                    .unwrap_or(u64::MAX),
            )
            .with_details(details),
        );
    }
    let verdict = "NATIVE_GAME_RUNTIME_NOT_TESTED";
    events.push(DiagnosticEvent::new("pipeline", "integration_not_tested", &correlation_id, "dekan-native", DiagnosticOpStatus::NotVerified, utc_now_micros())
        .with_transition(DiagnosticTransition::IntegrationRuntimeNotTested)
        .with_error(verdict.to_string()).with_details(serde_json::json!({
            "message": "Artifact inventory completed; this diagnostic does not start a game session", "game_load_attempted": false, "signature_rejection": "NOT_VERIFIED", "platform_restriction": "NOT_VERIFIED", "runtime_operations": "NOT_EXECUTED", "owned_pid": std::process::id()
        })));
    DiagnosticSummary {
        verdict: verdict.to_string(), correlation_id, build_version: env!("CARGO_PKG_VERSION").to_string(), build_revision: "unattested-local-build".to_string(),
        production_provider: selected.to_string(), target_champion: None, target_skin: None, total_asset_count: 0,
        memory_usage_bytes: query_process_memory_usage(), ipc_latency_percentiles: None,
        total_pipeline_duration_ms: started.elapsed().as_millis().try_into().unwrap_or(u64::MAX), native_provider_available: available,
        native_provider_status: if available { "ArtifactsPresentRuntimeNotVerified" } else { "ArtifactsUnavailable" }.to_string(),
        game_process_detected: None, laboratory_engine_ready: false,
        missing_capabilities: vec!["Native game DLL loading, resource consumption and rendering have not been verified in a real match".to_string()], events,
    }
}
