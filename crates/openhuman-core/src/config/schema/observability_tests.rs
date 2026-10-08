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
fn trace_sharing_and_content_capture_require_consent() {
    assert!(!default_share_usage_data());
    assert!(!ObservabilityConfig::default().share_usage_data);
    assert!(!default_capture_content());
    assert!(!AgentTracingConfig::default().capture_content);
}

#[test]
fn deserialize_missing_optional_fields_uses_defaults() {
    let cfg: ObservabilityConfig = serde_json::from_value(json!({})).unwrap();
    assert!(cfg.analytics_enabled, "analytics default must be true");
    assert!(
        !cfg.share_usage_data,
        "trace sharing requires an explicit opt-in"
    );
    // The local exporter stays opt-in and vendor-neutral by default.
    assert!(
        !cfg.agent_tracing.enabled,
        "local tracing exporter is opt-in"
    );
    assert_eq!(cfg.agent_tracing.backend, AgentTracingBackend::Otel);
    assert!(cfg.agent_tracing.export_path.is_none());
    assert!(
        !cfg.agent_tracing.capture_content,
        "content capture requires a separate opt-in"
    );
}

#[test]
fn capture_content_can_be_enabled_independently() {
    assert!(!AgentTracingConfig::default().capture_content);
    let cfg: ObservabilityConfig = serde_json::from_value(json!({
        "agent_tracing": { "capture_content": true }
    }))
    .unwrap();
    assert!(cfg.agent_tracing.capture_content);
    assert!(!cfg.share_usage_data);
}

#[test]
fn trace_sharing_can_be_enabled_with_metadata_only() {
    let cfg: ObservabilityConfig =
        serde_json::from_value(json!({ "share_usage_data": true })).unwrap();
    assert!(cfg.share_usage_data);
    assert!(!cfg.agent_tracing.capture_content);
}

#[test]
fn explicit_trace_consent_round_trips() {
    let cfg: ObservabilityConfig = serde_json::from_value(json!({
        "share_usage_data": true,
        "agent_tracing": { "capture_content": true }
    }))
    .unwrap();
    let saved = serde_json::to_string(&cfg).unwrap();
    let restored: ObservabilityConfig = serde_json::from_str(&saved).unwrap();
    assert!(restored.share_usage_data);
    assert!(restored.agent_tracing.capture_content);
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
        share_usage_data: false,
        agent_tracing: AgentTracingConfig::default(),
    };
    let s = serde_json::to_string(&original).unwrap();
    let back: ObservabilityConfig = serde_json::from_str(&s).unwrap();
    assert_eq!(
        back.sentry_dsn.as_deref(),
        Some("https://token@sentry.io/1")
    );
    assert!(!back.analytics_enabled);
    assert!(!back.share_usage_data);
}
