use super::*;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tinyagents_harness::observability::trace_export::{SpanKind, SpanStatus};

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

/// A sink that records what reached it.
#[derive(Default)]
struct RecordingSink {
    ready: bool,
    readiness_checks: AtomicUsize,
    pushed: AtomicUsize,
}

#[async_trait::async_trait]
impl super::super::TraceSink for RecordingSink {
    async fn ready(&self, _config: &Config) -> bool {
        self.readiness_checks.fetch_add(1, Ordering::SeqCst);
        self.ready
    }

    async fn push(&self, _config: &Config, spans: &[TraceSpan]) -> Result<(), String> {
        self.pushed.fetch_add(spans.len(), Ordering::SeqCst);
        Err("recorded".to_string())
    }
}

#[tokio::test]
async fn usage_data_spans_reach_registered_sinks_only_when_sharing() {
    let sink = Arc::new(RecordingSink::default());
    super::super::register_trace_sink("test-recording", sink.clone());
    let mut config = Config::default();
    config.observability.share_usage_data = false;
    export_run_trace(&config, &[file_fixture_span()]).await;
    assert_eq!(sink.pushed.load(Ordering::SeqCst), 0, "sharing off");

    config.observability.share_usage_data = true;
    export_run_trace(&config, &[file_fixture_span()]).await;
    assert_eq!(
        sink.pushed.load(Ordering::SeqCst),
        1,
        "a sink error is swallowed after the push"
    );
    export_run_trace(&config, &[]).await;
    assert_eq!(sink.pushed.load(Ordering::SeqCst), 1, "no spans, no push");
    assert!(super::super::unregister_trace_sink("test-recording"));
}

#[tokio::test]
async fn child_export_obeys_sharing_and_readiness_gates_before_reading() {
    let dispatch = crate::core::runtime::DispatchCtx::ambient();
    let sink = Arc::new(RecordingSink::default());
    super::super::register_trace_sink("test-unready", sink.clone());
    let mut config = Config::default();
    config.observability.share_usage_data = false;
    crate::core::observability::progress_tracing::export_subagent_journal_trace(
        &dispatch,
        &config,
        "missing-child-run",
        None,
        "worker",
        "task-1",
    )
    .await;
    assert_eq!(
        sink.readiness_checks.load(Ordering::SeqCst),
        0,
        "sharing off skips before any sink is asked"
    );

    config.observability.share_usage_data = true;
    crate::core::observability::progress_tracing::export_subagent_journal_trace(
        &dispatch,
        &config,
        "missing-child-run",
        None,
        "worker",
        "task-1",
    )
    .await;
    assert_eq!(sink.readiness_checks.load(Ordering::SeqCst), 1);
    assert_eq!(
        sink.pushed.load(Ordering::SeqCst),
        0,
        "an unready sink skips the journal read and the push"
    );
    assert!(super::super::unregister_trace_sink("test-unready"));
}
