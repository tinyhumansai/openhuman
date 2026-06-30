//! Observability (logging, metrics, tracing) configuration.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ObservabilityConfig {
    /// Sentry DSN for error reporting. Overridden by the
    /// `OPENHUMAN_CORE_SENTRY_DSN` env var (or its legacy alias
    /// `OPENHUMAN_SENTRY_DSN`).
    #[serde(default)]
    pub sentry_dsn: Option<String>,

    /// Whether anonymized analytics and error reporting is enabled.
    /// Defaults to `true`. Users can disable via settings or CLI.
    #[serde(default = "default_analytics_enabled")]
    pub analytics_enabled: bool,

    /// Opt-in structured tracing export for agent runs (issue #3886).
    /// Off by default; see [`AgentTracingConfig`].
    #[serde(default)]
    pub agent_tracing: AgentTracingConfig,
}

fn default_analytics_enabled() -> bool {
    true
}

/// Destination format for the agent tracing export. Vendor-neutral
/// OpenTelemetry by default; Langfuse is offered for teams already on it.
/// Both share the same span model — only the serialized envelope differs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "lowercase")]
pub enum AgentTracingBackend {
    /// OpenTelemetry-style spans (vendor-neutral). Default.
    #[default]
    Otel,
    /// Langfuse-style observations.
    Langfuse,
}

/// Opt-in structured tracing export driven by the agent progress channel.
///
/// When [`Self::enabled`] is `true`, agent runs emit OpenTelemetry/Langfuse-
/// style spans (turn → iteration → tool call / subagent) correlated by session
/// id with user attribution, appended as NDJSON to [`Self::export_path`].
///
/// Off by default and intentionally side-effect-free when disabled. Spans
/// carry only metadata (names, counts, timings, token/cost figures) — never
/// prompt text, tool arguments, streamed deltas, error text, or file paths —
/// honoring the project's "never log secrets or full PII" rule.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct AgentTracingConfig {
    /// Master switch. Off by default.
    pub enabled: bool,

    /// Serialized span envelope to emit. Defaults to OpenTelemetry.
    pub backend: AgentTracingBackend,

    /// Absolute path of the NDJSON file spans are appended to. When unset,
    /// spans are emitted to the application log at `info` level instead, so
    /// the export still works on read-only or sandboxed deployments.
    pub export_path: Option<String>,
}

impl Default for AgentTracingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            backend: AgentTracingBackend::Otel,
            export_path: None,
        }
    }
}

impl Default for ObservabilityConfig {
    fn default() -> Self {
        Self {
            sentry_dsn: None,
            analytics_enabled: true,
            agent_tracing: AgentTracingConfig::default(),
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
        let cfg: ObservabilityConfig = serde_json::from_value(json!({})).unwrap();
        assert!(cfg.analytics_enabled, "analytics default must be true");
        assert!(
            !cfg.agent_tracing.enabled,
            "agent tracing must default off (opt-in)"
        );
        assert_eq!(cfg.agent_tracing.backend, AgentTracingBackend::Otel);
        assert!(cfg.agent_tracing.export_path.is_none());
    }

    #[test]
    fn deserialize_agent_tracing_block() {
        let cfg: ObservabilityConfig = serde_json::from_value(json!({
            "agent_tracing": {
                "enabled": true,
                "backend": "langfuse",
                "export_path": "/var/log/openhuman/spans.ndjson"
            }
        }))
        .unwrap();
        assert!(cfg.agent_tracing.enabled);
        assert_eq!(cfg.agent_tracing.backend, AgentTracingBackend::Langfuse);
        assert_eq!(
            cfg.agent_tracing.export_path.as_deref(),
            Some("/var/log/openhuman/spans.ndjson")
        );
    }

    #[test]
    fn agent_tracing_backend_defaults_to_otel() {
        assert_eq!(AgentTracingBackend::default(), AgentTracingBackend::Otel);
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
            agent_tracing: AgentTracingConfig::default(),
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
