use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DiagnosticOpStatus {
    Requested,
    Started,
    Acknowledged,
    Completed,
    Failed,
    NotImplemented,
    NotVerified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DiagnosticTransition {
    SkinSelected,
    ProviderSelected,
    OverlayBuildStarted,
    OverlayBuildCompleted,
    NativeHostStarted,
    NativeDllLoaded,
    IpcConnected,
    OverlayConfigured,
    ResourceRequested,
    ResourceResolved,
    ResourceConsumed,
    IntegrationUnavailable,
    IntegrationRuntimeNotTested,
    OperationFailed,
    ShutdownCompleted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticEvent {
    pub timestamp_utc: String,
    pub component: String,
    pub event_name: String,
    pub transition: Option<DiagnosticTransition>,
    pub correlation_id: String,
    pub provider: String,
    pub operation_status: DiagnosticOpStatus,
    pub duration_ms: Option<u64>,
    pub error_code: Option<String>,
    pub details: serde_json::Value,
}

impl DiagnosticEvent {
    pub fn new(
        component: &str,
        event_name: &str,
        correlation_id: &str,
        provider: &str,
        operation_status: DiagnosticOpStatus,
        timestamp_utc: String,
    ) -> Self {
        Self {
            timestamp_utc,
            component: component.to_string(),
            event_name: event_name.to_string(),
            transition: None,
            correlation_id: correlation_id.to_string(),
            provider: provider.to_string(),
            operation_status,
            duration_ms: None,
            error_code: None,
            details: serde_json::Value::Null,
        }
    }

    #[must_use]
    pub fn with_transition(mut self, transition: DiagnosticTransition) -> Self {
        self.transition = Some(transition);
        self
    }

    #[must_use]
    pub fn with_duration(mut self, ms: u64) -> Self {
        self.duration_ms = Some(ms);
        self
    }

    #[must_use]
    pub fn with_error(mut self, code: String) -> Self {
        self.error_code = Some(code);
        self
    }

    #[must_use]
    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = details;
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpcLatencyPercentiles {
    pub min_us: f64,
    pub p50_us: f64,
    pub p95_us: f64,
    pub p99_us: f64,
    pub max_us: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticSummary {
    pub verdict: String,
    pub correlation_id: String,
    pub build_version: String,
    pub build_revision: String,
    pub production_provider: String,
    pub target_champion: Option<String>,
    pub target_skin: Option<String>,
    pub total_asset_count: usize,
    pub memory_usage_bytes: u64,
    pub ipc_latency_percentiles: Option<IpcLatencyPercentiles>,
    pub total_pipeline_duration_ms: u64,
    pub native_provider_available: bool,
    pub native_provider_status: String,
    pub game_process_detected: Option<u32>,
    pub laboratory_engine_ready: bool,
    pub missing_capabilities: Vec<String>,
    pub events: Vec<DiagnosticEvent>,
}
