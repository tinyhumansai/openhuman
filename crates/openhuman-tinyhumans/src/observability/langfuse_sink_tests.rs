use super::*;
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
    let config = Config {
        api_url: Some("https://external.example.invalid/api".into()),
        ..Config::default()
    };
    assert!(push_spans(&config, &[turn_span()]).await.is_ok());
    assert!(push_spans(&config, &[]).await.is_ok());
}

#[tokio::test]
async fn an_external_backend_is_not_ready() {
    let config = Config {
        api_url: Some("https://external.invalid/api".into()),
        ..Config::default()
    };
    assert!(!LangfuseSink.ready(&config).await);
    assert!(LangfuseSink.push(&config, &[turn_span()]).await.is_ok());
}
