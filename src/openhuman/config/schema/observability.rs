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

    /// Service name reported to the OTel collector. Defaults to "openhuman".
    #[serde(default)]
    pub otel_service_name: Option<String>,

    /// Sentry DSN for error reporting. Overridden by `OPENHUMAN_SENTRY_DSN` env var.
    #[serde(default)]
    pub sentry_dsn: Option<String>,

    /// Whether anonymized analytics and error reporting is enabled.
    /// Defaults to `true`. Users can disable via settings or CLI.
    #[serde(default = "default_analytics_enabled")]
    pub analytics_enabled: bool,
}

fn default_analytics_enabled() -> bool {
    true
}

impl Default for ObservabilityConfig {
    fn default() -> Self {
        Self {
            backend: "none".into(),
            otel_endpoint: None,
            otel_service_name: None,
            sentry_dsn: None,
            analytics_enabled: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn default_disables_backend_and_enables_analytics() {
        let cfg = ObservabilityConfig::default();
        assert_eq!(cfg.backend, "none");
        assert!(cfg.otel_endpoint.is_none());
        assert!(cfg.otel_service_name.is_none());
        assert!(cfg.sentry_dsn.is_none());
        assert!(cfg.analytics_enabled);
    }

    #[test]
    fn default_analytics_enabled_helper_returns_true() {
        assert!(default_analytics_enabled());
    }

    #[test]
    fn deserialize_missing_optional_fields_uses_defaults() {
        let cfg: ObservabilityConfig = serde_json::from_value(json!({
            "backend": "log"
        }))
        .unwrap();
        assert_eq!(cfg.backend, "log");
        assert!(cfg.otel_endpoint.is_none());
        assert!(cfg.analytics_enabled, "analytics default must be true");
    }

    #[test]
    fn deserialize_respects_explicit_analytics_flag() {
        let cfg: ObservabilityConfig = serde_json::from_value(json!({
            "backend": "otel",
            "analytics_enabled": false
        }))
        .unwrap();
        assert!(!cfg.analytics_enabled);
    }

    #[test]
    fn round_trip_preserves_all_fields() {
        let original = ObservabilityConfig {
            backend: "otel".into(),
            otel_endpoint: Some("http://localhost:4318".into()),
            otel_service_name: Some("openhuman-test".into()),
            sentry_dsn: Some("https://token@sentry.io/1".into()),
            analytics_enabled: false,
        };
        let s = serde_json::to_string(&original).unwrap();
        let back: ObservabilityConfig = serde_json::from_str(&s).unwrap();
        assert_eq!(back.backend, "otel");
        assert_eq!(back.otel_endpoint.as_deref(), Some("http://localhost:4318"));
        assert_eq!(back.otel_service_name.as_deref(), Some("openhuman-test"));
        assert!(back.sentry_dsn.is_some());
        assert!(!back.analytics_enabled);
    }
}
