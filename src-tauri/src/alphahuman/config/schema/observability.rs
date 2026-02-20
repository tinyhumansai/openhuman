//! Observability (logging, metrics, tracing) configuration.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ObservabilityConfig {
    /// "none" | "log" | "prometheus" | "otel"
    pub backend: String,

    /// OTLP endpoint (e.g. "http://localhost:4318"). Only used when backend = "otel".
    #[serde(default)]
    pub otel_endpoint: Option<String>,

    /// Service name reported to the OTel collector. Defaults to "alphahuman".
    #[serde(default)]
    pub otel_service_name: Option<String>,
}

impl Default for ObservabilityConfig {
    fn default() -> Self {
        Self {
            backend: "none".into(),
            otel_endpoint: None,
            otel_service_name: None,
        }
    }
}
