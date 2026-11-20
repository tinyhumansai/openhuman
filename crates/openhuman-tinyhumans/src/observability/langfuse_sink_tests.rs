use super::*;
use openhuman_embed::__host::credentials::api_key::store_api_key;
use std::collections::BTreeMap;
use tinyagents_harness::observability::trace_export::{SpanKind, SpanStatus};

#[test]
fn partial_success_error_message_is_rejected_even_when_rejected_count_is_zero() {
    let response = serde_json::json!({
        "partialSuccess": { "rejectedSpans": "0", "errorMessage": "receiver warning" }
    });
    assert!(reject_partial_success(&response).is_err());
}

#[test]
fn partial_success_positive_rejected_count_is_rejected() {
    let response = serde_json::json!({
        "partialSuccess": { "rejectedSpans": "2", "errorMessage": "some spans rejected" }
    });
    assert!(reject_partial_success(&response).is_err());
}

#[test]
fn partial_success_positive_rejected_count_without_message_is_rejected() {
    let response = serde_json::json!({
        "partialSuccess": { "rejectedSpans": "2" }
    });
    assert!(reject_partial_success(&response).is_err());
}

#[test]
fn partial_success_zero_rejected_count_without_message_is_accepted() {
    let response = serde_json::json!({
        "partialSuccess": { "rejectedSpans": "0" }
    });
    assert!(reject_partial_success(&response).is_ok());
}

#[test]
fn partial_success_unreadable_rejected_count_is_rejected() {
    let response = serde_json::json!({
        "partialSuccess": { "rejectedSpans": "two" }
    });
    assert!(reject_partial_success(&response).is_err());
}

#[test]
fn empty_partial_success_response_is_accepted() {
    assert!(reject_partial_success(&serde_json::json!({ "partialSuccess": {} })).is_ok());
    assert!(reject_partial_success(&serde_json::json!({})).is_ok());
}

fn turn_span() -> TraceSpan {
    TraceSpan {
        trace_id: "thread-one:turn-one".into(),
        span_id: "root".into(),
        parent_span_id: None,
        name: "agent.turn".into(),
        kind: SpanKind::Turn,
        start_unix_ms: 1_000,
        end_unix_ms: Some(2_000),
        status: SpanStatus::Ok,
        attributes: BTreeMap::new(),
        input: None,
        output: None,
    }
}

#[tokio::test]
async fn external_backend_is_skipped_before_session_lookup() {
    let mut config = Config::default();
    config.resolved_endpoints.tracing_otlp_url =
        Some("https://external.example.invalid/v1/traces".into());
    config.resolved_endpoints.deployment_environment = None;
    assert_eq!(
        environment_for_base(&config, "https://external.example.invalid/v1/traces"),
        "external"
    );
    assert!(push_spans(&config, &[turn_span()]).await.is_ok());
    assert!(push_spans(&config, &[]).await.is_ok());
}

#[tokio::test]
async fn an_external_backend_is_not_ready() {
    let mut config = Config::default();
    config.resolved_endpoints.tracing_otlp_url = Some("https://external.invalid/v1/traces".into());
    config.resolved_endpoints.deployment_environment = None;
    assert_eq!(
        environment_for_base(&config, "https://external.invalid/v1/traces"),
        "external"
    );
    assert!(!LangfuseSink.ready(&config).await);
    assert!(LangfuseSink.push(&config, &[turn_span()]).await.is_ok());
}

#[tokio::test]
async fn api_key_credential_reaches_the_langfuse_push() {
    // An API-key host must get past the credential gate exactly like a
    // session host. The OTLP endpoint here is a closed loopback port, so the
    // push itself fails with a connection error — that failure is the proof
    // the gate let it through: before the fix this returned Ok(()) without
    // ever attempting a request.
    let dir = tempfile::tempdir().expect("tempdir");
    let mut config = Config {
        config_path: dir.path().join("config.toml"),
        workspace_dir: dir.path().join("workspace"),
        action_dir: dir.path().join("workspace"),
        ..Default::default()
    };
    config.secrets.encrypt = false;
    store_api_key(&config, "th_test_key")
        .await
        .expect("store key");
    config.resolved_endpoints.control_plane_url = Some("http://127.0.0.1:7788".into());
    config.resolved_endpoints.tracing_otlp_url =
        Some("http://127.0.0.1:1/traces/otel/v1/traces".into());
    assert!(push_spans(&config, &[turn_span()]).await.is_err());
}
