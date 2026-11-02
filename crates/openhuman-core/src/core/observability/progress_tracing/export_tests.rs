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

fn file_fixture_span() -> TraceSpan {
    TraceSpan {
        trace_id: "writer-trace".into(),
        span_id: "writer-root".into(),
        parent_span_id: None,
        name: "agent.turn".into(),
        kind: SpanKind::Turn,
        start_unix_ms: 1_000,
        end_unix_ms: Some(1_100),
        status: SpanStatus::Ok,
        attributes: BTreeMap::new(),
        input: None,
        output: None,
    }
}

#[test]
fn local_export_appends_valid_selected_ndjson_envelopes() {
    let scratch = tempfile::tempdir().expect("isolated trace output");
    let path = scratch.path().join("trace.ndjson");
    let mut config = AgentTracingConfig {
        enabled: true,
        export_path: Some(path.to_string_lossy().into_owned()),
        ..Default::default()
    };

    export_spans(&config, &[file_fixture_span()]);
    config.backend = AgentTracingBackend::Langfuse;
    export_spans(&config, &[file_fixture_span()]);

    let lines: Vec<_> = std::fs::read_to_string(&path)
        .expect("trace file was created")
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("valid NDJSON"))
        .collect();
    assert_eq!(lines.len(), 2, "each call appends exactly one span line");
    assert_eq!(
        lines[0]["trace_id"], "writer-trace",
        "OTel envelope selected"
    );
    assert_eq!(
        lines[1]["body"]["trace_id"], "writer-trace",
        "Langfuse envelope selected"
    );
}

#[test]
fn disabled_local_export_and_log_only_export_are_safe() {
    let scratch = tempfile::tempdir().expect("isolated trace output");
    let path = scratch.path().join("disabled.ndjson");
    let config = AgentTracingConfig {
        enabled: false,
        export_path: Some(path.to_string_lossy().into_owned()),
        ..Default::default()
    };
    export_spans(&config, &[file_fixture_span()]);
    assert!(
        !path.exists(),
        "disabled local tracing has no filesystem effect"
    );

    let config = AgentTracingConfig {
        enabled: true,
        export_path: None,
        ..Default::default()
    };
    export_spans(&config, &[file_fixture_span()]);
}

#[tokio::test]
async fn external_backend_is_skipped_before_session_lookup() {
    let mut config = Config::default();
    config.api_url = Some("https://external.example.invalid/openai/v1".into());
    let spans = [TraceSpan {
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
    }];
    assert!(push_spans(&config, &spans).await.is_ok());
}

#[tokio::test]
async fn child_export_obeys_sharing_and_readiness_gates_before_reading() {
    let dispatch = crate::core::runtime::DispatchCtx::ambient();
    let mut config = Config::default();
    crate::core::observability::progress_tracing::export_subagent_journal_trace(
        &dispatch,
        &config,
        "missing-child-run",
        None,
        "worker",
        "task-1",
    )
    .await;

    config.observability.share_usage_data = true;
    config.api_url = Some("https://external.invalid/api".into());
    assert!(!crate::core::observability::telemetry::langfuse::journal_push_ready(&config).await);
    crate::core::observability::progress_tracing::export_subagent_journal_trace(
        &dispatch,
        &config,
        "missing-child-run",
        None,
        "worker",
        "task-1",
    )
    .await;
}
