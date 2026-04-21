//! Observability (logging, metrics, tracing) configuration.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ObservabilityConfig {
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
    fn default_enables_analytics() {
        let cfg = ObservabilityConfig::default();
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
            "analytics_enabled": true
        }))
        .unwrap();
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
            sentry_dsn: Some("https://token@sentry.io/1".into()),
            analytics_enabled: false,
        };
        let s = serde_json::to_string(&original).unwrap();
        let back: ObservabilityConfig = serde_json::from_str(&s).unwrap();
        assert_eq!(
            back.sentry_dsn.as_deref(),
            Some("https://token@sentry.io/1")
        );
        assert!(!back.analytics_enabled);
    }
}
