//! Unit tests for the structured tracing export (issue #3886).

use super::*;
use crate::openhuman::agent::progress::AgentProgress;
use crate::openhuman::config::schema::{
    AgentTracingBackend, AgentTracingConfig, ObservabilityConfig,
};

fn ctx() -> TraceContext {
    TraceContext::new("sess-42", Some("client-7".to_string()))
}

fn collect(events: &[(AgentProgress, u64)]) -> SpanCollector {
    let mut c = SpanCollector::new(ctx());
    for (event, ts) in events {
        c.record(event, *ts);
    }
    c
}

fn find<'a>(spans: &'a [TraceSpan], name: &str) -> &'a TraceSpan {
    spans
        .iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| panic!("no span named {name:?} in {:?}", names(spans)))
}

fn names(spans: &[TraceSpan]) -> Vec<String> {
    spans.iter().map(|s| s.name.clone()).collect()
}

// ── config ────────────────────────────────────────────────────────────────

#[test]
fn local_tracing_exporter_is_off_by_default() {
    let cfg = AgentTracingConfig::default();
    assert!(!cfg.enabled, "local exporter is opt-in");
    assert_eq!(cfg.backend, AgentTracingBackend::Otel);
    assert!(cfg.export_path.is_none());
}

#[test]
fn observability_default_shares_usage_but_keeps_local_exporter_off() {
    let obs = ObservabilityConfig::default();
    assert!(obs.share_usage_data, "usage-data sharing is on by default");
    assert!(!obs.agent_tracing.enabled, "local exporter stays opt-in");
}

#[test]
fn tracing_config_round_trips_through_json() {
    let cfg = AgentTracingConfig {
        enabled: true,
        backend: AgentTracingBackend::Langfuse,
        export_path: Some("/tmp/spans.ndjson".to_string()),
        capture_content: false,
    };
    let s = serde_json::to_string(&cfg).unwrap();
    let back: AgentTracingConfig = serde_json::from_str(&s).unwrap();
    assert!(back.enabled);
    assert_eq!(back.backend, AgentTracingBackend::Langfuse);
    assert_eq!(back.export_path.as_deref(), Some("/tmp/spans.ndjson"));
    // lowercase serde rename.
    assert!(s.contains("\"langfuse\""));
}

// ── parent turn ─────────────────────────────────────────────────────────────

fn tool_started(call_id: &str, tool: &str, iter: u32) -> AgentProgress {
    AgentProgress::ToolCallStarted {
        call_id: call_id.to_string(),
        tool_name: tool.to_string(),
        arguments: serde_json::json!({"secret": "do-not-export"}),
        iteration: iter,
        display_label: None,
        display_detail: None,
    }
}

fn tool_completed(
    call_id: &str,
    tool: &str,
    success: bool,
    chars: usize,
    elapsed: u64,
) -> AgentProgress {
    AgentProgress::ToolCallCompleted {
        call_id: call_id.to_string(),
        tool_name: tool.to_string(),
        success,
        output_chars: chars,
        output: String::new(),
        arguments: None,
        elapsed_ms: elapsed,
        iteration: 1,
        failure: None,
    }
}

#[test]
fn full_turn_builds_correlated_span_tree() {
    let mut c = collect(&[
        (AgentProgress::TurnStarted, 1_000),
        (
            AgentProgress::IterationStarted {
                iteration: 1,
                max_iterations: 5,
            },
            1_010,
        ),
        (tool_started("call-1", "web_search", 1), 1_020),
        (
            tool_completed("call-1", "web_search", true, 321, 200),
            1_020,
        ),
        (
            AgentProgress::TurnCostUpdated {
                model: "claude-opus-4-8".to_string(),
                iteration: 1,
                input_tokens: 1_200,
                output_tokens: 480,
                cached_input_tokens: 64,
                total_usd: 0.042,
            },
            1_030,
        ),
        (AgentProgress::TurnCompleted { iterations: 1 }, 1_040),
    ]);
    c.finish(2_000);
    let spans = c.spans();

    // Root turn: trace = session id, user attribution present.
    let turn = find(spans, "agent.turn");
    assert_eq!(turn.kind, SpanKind::Turn);
    assert_eq!(turn.trace_id, "sess-42");
    assert!(turn.parent_span_id.is_none());
    assert_eq!(turn.attributes["session.id"], serde_json::json!("sess-42"));
    assert_eq!(turn.attributes["user.id"], serde_json::json!("client-7"));
    // Cost / usage attributes ride on the root.
    assert_eq!(
        turn.attributes["gen_ai.usage.input_tokens"],
        serde_json::json!(1_200)
    );
    assert_eq!(
        turn.attributes["gen_ai.usage.output_tokens"],
        serde_json::json!(480)
    );
    assert_eq!(
        turn.attributes["gen_ai.usage.cached_input_tokens"],
        serde_json::json!(64)
    );
    assert_eq!(
        turn.attributes["gen_ai.request.model"],
        serde_json::json!("claude-opus-4-8")
    );
    assert_eq!(turn.attributes["agent.iterations"], serde_json::json!(1));
    assert_eq!(turn.status, SpanStatus::Ok);
    assert!(turn.attributes.get("gen_ai.usage.cost_usd").is_some());

    // Iteration parented to the turn.
    let iter = find(spans, "agent.iteration#1");
    assert_eq!(iter.kind, SpanKind::Iteration);
    assert_eq!(iter.parent_span_id.as_deref(), Some(turn.span_id.as_str()));
    assert_eq!(
        iter.attributes["agent.max_iterations"],
        serde_json::json!(5)
    );

    // Tool parented to the iteration.
    let tool = find(spans, "tool.web_search");
    assert_eq!(tool.kind, SpanKind::Tool);
    assert_eq!(tool.parent_span_id.as_deref(), Some(iter.span_id.as_str()));
    assert_eq!(tool.status, SpanStatus::Ok);
    assert_eq!(tool.attributes["tool.output_chars"], serde_json::json!(321));
    // end = start + elapsed_ms.
    assert_eq!(tool.duration_ms(), Some(200));

    // Everything sealed.
    assert!(spans.iter().all(|s| s.end_unix_ms.is_some()));
}

#[test]
fn failed_tool_marks_error_status() {
    let mut c = collect(&[
        (AgentProgress::TurnStarted, 0),
        (
            AgentProgress::IterationStarted {
                iteration: 1,
                max_iterations: 3,
            },
            1,
        ),
        (tool_started("c1", "shell", 1), 2),
        (tool_completed("c1", "shell", false, 12, 5), 2),
    ]);
    c.finish(100);
    let tool = find(c.spans(), "tool.shell");
    assert_eq!(tool.status, SpanStatus::Error);
    assert_eq!(tool.attributes["tool.success"], serde_json::json!(false));
}

#[test]
fn iteration_started_closes_the_previous_iteration() {
    let c = collect(&[
        (AgentProgress::TurnStarted, 0),
        (
            AgentProgress::IterationStarted {
                iteration: 1,
                max_iterations: 3,
            },
            10,
        ),
        (
            AgentProgress::IterationStarted {
                iteration: 2,
                max_iterations: 3,
            },
            20,
        ),
    ]);
    let first = find(c.spans(), "agent.iteration#1");
    assert_eq!(
        first.end_unix_ms,
        Some(20),
        "iter#1 closes when iter#2 opens"
    );
    let second = find(c.spans(), "agent.iteration#2");
    assert!(
        second.end_unix_ms.is_none(),
        "iter#2 still open until finish"
    );
}

// ── subagents ───────────────────────────────────────────────────────────────

fn spawn(task: &str, display: &str) -> AgentProgress {
    AgentProgress::SubagentSpawned {
        agent_id: "researcher".to_string(),
        task_id: task.to_string(),
        mode: "typed".to_string(),
        dedicated_thread: true,
        prompt_chars: 256,
        prompt: "delegated prompt".to_string(),
        worker_thread_id: Some("worker-abc".to_string()),
        display_name: Some(display.to_string()),
    }
}

#[test]
fn subagent_lifecycle_nests_under_the_turn() {
    let mut c = collect(&[
        (AgentProgress::TurnStarted, 0),
        (
            AgentProgress::IterationStarted {
                iteration: 1,
                max_iterations: 5,
            },
            5,
        ),
        (spawn("task-1", "Researcher"), 10),
        (
            AgentProgress::SubagentIterationStarted {
                agent_id: "researcher".to_string(),
                task_id: "task-1".to_string(),
                iteration: 1,
                max_iterations: 8,
                extended_policy: true,
            },
            20,
        ),
        (
            AgentProgress::SubagentToolCallStarted {
                agent_id: "researcher".to_string(),
                task_id: "task-1".to_string(),
                call_id: "sc-1".to_string(),
                tool_name: "read_file".to_string(),
                arguments: serde_json::Value::Null,
                iteration: 1,
                display_label: None,
                display_detail: None,
            },
            30,
        ),
        (
            AgentProgress::SubagentToolCallCompleted {
                agent_id: "researcher".to_string(),
                task_id: "task-1".to_string(),
                call_id: "sc-1".to_string(),
                tool_name: "read_file".to_string(),
                success: true,
                output_chars: 99,
                output: "file contents".to_string(),
                arguments: None,
                elapsed_ms: 40,
                iteration: 1,
                failure: None,
            },
            30,
        ),
        (
            AgentProgress::SubagentCompleted {
                agent_id: "researcher".to_string(),
                task_id: "task-1".to_string(),
                elapsed_ms: 500,
                iterations: 3,
                output_chars: 1024,
                output: String::new(),
                worktree_path: Some("/private/should/not/leak".to_string()),
                changed_files: vec!["secret_file.rs".to_string()],
                dirty_status: Some(true),
            },
            40,
        ),
    ]);
    c.finish(1_000);
    let spans = c.spans();

    let iter = find(spans, "agent.iteration#1");
    let sub = find(spans, "subagent.Researcher");
    assert_eq!(sub.kind, SpanKind::Subagent);
    assert_eq!(sub.parent_span_id.as_deref(), Some(iter.span_id.as_str()));
    assert_eq!(
        sub.attributes["subagent.task_id"],
        serde_json::json!("task-1")
    );
    assert_eq!(sub.attributes["subagent.iterations"], serde_json::json!(3));
    assert_eq!(
        sub.attributes["subagent.output_chars"],
        serde_json::json!(1024)
    );
    assert_eq!(sub.duration_ms(), Some(500));

    let child_iter = find(spans, "subagent.iteration#1");
    assert_eq!(child_iter.kind, SpanKind::SubagentIteration);
    assert_eq!(
        child_iter.parent_span_id.as_deref(),
        Some(sub.span_id.as_str())
    );
    assert_eq!(
        child_iter.attributes["agent.extended_policy"],
        serde_json::json!(true)
    );

    let child_tool = find(spans, "tool.read_file");
    assert_eq!(
        child_tool.parent_span_id.as_deref(),
        Some(child_iter.span_id.as_str())
    );
    assert_eq!(child_tool.status, SpanStatus::Ok);

    // Worktree paths / changed file names must never be exported.
    let blob = serde_json::to_string(spans).unwrap();
    assert!(!blob.contains("should/not/leak"));
    assert!(!blob.contains("secret_file.rs"));
}

#[test]
fn subagent_failure_records_error_without_raw_text() {
    let mut c = collect(&[
        (AgentProgress::TurnStarted, 0),
        (spawn("task-9", "Coder"), 5),
        (
            AgentProgress::SubagentFailed {
                agent_id: "coder".to_string(),
                task_id: "task-9".to_string(),
                error: "API key sk-secret-123 leaked in stacktrace".to_string(),
            },
            10,
        ),
    ]);
    c.finish(50);
    let sub = find(c.spans(), "subagent.Coder");
    assert_eq!(sub.status, SpanStatus::Error);
    assert_eq!(sub.attributes["error"], serde_json::json!(true));
    assert!(sub.attributes.get("error.length").is_some());

    let blob = serde_json::to_string(c.spans()).unwrap();
    assert!(
        !blob.contains("sk-secret-123"),
        "raw error text must not leak"
    );
}

#[test]
fn unknown_subagent_task_ids_are_ignored() {
    // Completion/tool events with no matching spawn must not panic or spawn.
    let mut c = SpanCollector::new(ctx());
    c.record(&AgentProgress::TurnStarted, 0);
    c.record(
        &AgentProgress::SubagentCompleted {
            agent_id: "x".to_string(),
            task_id: "ghost".to_string(),
            elapsed_ms: 1,
            iterations: 1,
            output_chars: 1,
            output: String::new(),
            worktree_path: None,
            changed_files: vec![],
            dirty_status: None,
        },
        10,
    );
    // Only the turn span exists.
    assert_eq!(names(c.spans()), vec!["agent.turn".to_string()]);
}

// ── privacy ─────────────────────────────────────────────────────────────────

#[test]
fn content_bearing_events_produce_no_spans_and_no_leak() {
    let c = collect(&[
        (AgentProgress::TurnStarted, 0),
        (
            AgentProgress::TextDelta {
                delta: "TOP-SECRET-REPLY".to_string(),
                iteration: 1,
            },
            1,
        ),
        (
            AgentProgress::ThinkingDelta {
                delta: "secret reasoning".to_string(),
                iteration: 1,
            },
            2,
        ),
        (
            AgentProgress::ToolCallArgsDelta {
                call_id: "c".to_string(),
                tool_name: "shell".to_string(),
                delta: "rm -rf /secret".to_string(),
                iteration: 1,
            },
            3,
        ),
    ]);
    // Only the lazily-opened turn span, nothing from the deltas.
    assert_eq!(names(c.spans()), vec!["agent.turn".to_string()]);
    let blob = serde_json::to_string(c.spans()).unwrap();
    assert!(!blob.contains("TOP-SECRET-REPLY"));
    assert!(!blob.contains("secret reasoning"));
    assert!(!blob.contains("rm -rf"));
}

#[test]
fn tool_arguments_are_never_serialized() {
    let mut c = collect(&[
        (AgentProgress::TurnStarted, 0),
        (tool_started("c1", "shell", 1), 1),
    ]);
    c.finish(10);
    let blob = serde_json::to_string(c.spans()).unwrap();
    assert!(
        !blob.contains("do-not-export"),
        "tool args must not be exported"
    );
}

// ── lazy root + finish ──────────────────────────────────────────────────────

#[test]
fn first_event_lazily_opens_the_turn_span() {
    // Stream that begins mid-flight (no TurnStarted) still correlates.
    let mut c = SpanCollector::new(ctx());
    c.record(
        &AgentProgress::IterationStarted {
            iteration: 4,
            max_iterations: 9,
        },
        100,
    );
    let turn = find(c.spans(), "agent.turn");
    assert_eq!(turn.trace_id, "sess-42");
    let iter = find(c.spans(), "agent.iteration#4");
    assert_eq!(iter.parent_span_id.as_deref(), Some(turn.span_id.as_str()));
}

#[test]
fn finish_seals_all_open_spans_idempotently() {
    let mut c = collect(&[
        (AgentProgress::TurnStarted, 0),
        (
            AgentProgress::IterationStarted {
                iteration: 1,
                max_iterations: 2,
            },
            5,
        ),
        (tool_started("c1", "x", 1), 6),
    ]);
    assert!(c.spans().iter().any(|s| s.end_unix_ms.is_none()));
    c.finish(99);
    assert!(c.spans().iter().all(|s| s.end_unix_ms.is_some()));
    // idempotent.
    c.finish(200);
    assert!(c
        .spans()
        .iter()
        .all(|s| s.end_unix_ms == Some(s.end_unix_ms.unwrap())));
}

#[test]
fn cost_update_before_turn_start_lazily_opens_root() {
    let mut c = SpanCollector::new(ctx());
    c.record(
        &AgentProgress::TurnCostUpdated {
            model: "m".to_string(),
            iteration: 1,
            input_tokens: 10,
            output_tokens: 5,
            cached_input_tokens: 0,
            total_usd: 0.001,
        },
        100,
    );
    let turn = find(c.spans(), "agent.turn");
    assert_eq!(
        turn.attributes["gen_ai.usage.input_tokens"],
        serde_json::json!(10)
    );
}

#[test]
fn trace_session_id_prefers_ui_session_else_thread() {
    assert_eq!(trace_session_id(Some(99), "thread-x"), "99");
    assert_eq!(trace_session_id(None, "thread-x"), "thread-x");
}

#[test]
fn no_user_attribution_omits_user_id() {
    let mut c = SpanCollector::new(TraceContext::new("anon-1", None));
    c.record(&AgentProgress::TurnStarted, 0);
    let turn = find(c.spans(), "agent.turn");
    assert!(turn.attributes.get("user.id").is_none());
    assert_eq!(turn.attributes["session.id"], serde_json::json!("anon-1"));
}

// ── serialization + export ──────────────────────────────────────────────────

fn one_turn_spans() -> Vec<TraceSpan> {
    let mut c = collect(&[
        (AgentProgress::TurnStarted, 0),
        (AgentProgress::TurnCompleted { iterations: 1 }, 10),
    ]);
    c.finish(10);
    c.into_spans()
}

#[test]
fn ndjson_otel_emits_one_line_per_span() {
    let spans = one_turn_spans();
    let out = spans_to_ndjson(AgentTracingBackend::Otel, &spans);
    assert_eq!(out.lines().count(), spans.len());
    // Bare OTel span body has the fields directly.
    let first: serde_json::Value = serde_json::from_str(out.lines().next().unwrap()).unwrap();
    assert_eq!(first["trace_id"], serde_json::json!("sess-42"));
    assert_eq!(first["kind"], serde_json::json!("turn"));
}

#[test]
fn ndjson_langfuse_wraps_each_span_in_an_observation_envelope() {
    let spans = one_turn_spans();
    let out = spans_to_ndjson(AgentTracingBackend::Langfuse, &spans);
    let first: serde_json::Value = serde_json::from_str(out.lines().next().unwrap()).unwrap();
    assert_eq!(first["type"], serde_json::json!("span-create"));
    assert_eq!(first["body"]["trace_id"], serde_json::json!("sess-42"));
}

#[test]
fn ndjson_empty_for_empty_slice() {
    assert!(spans_to_ndjson(AgentTracingBackend::Otel, &[]).is_empty());
}

#[test]
fn export_disabled_is_a_noop_and_writes_nothing() {
    let dir = std::env::temp_dir().join(format!("oh-trace-noop-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("spans.ndjson");
    let cfg = AgentTracingConfig {
        enabled: false,
        backend: AgentTracingBackend::Otel,
        export_path: Some(path.to_string_lossy().to_string()),
        capture_content: false,
    };
    export_spans(&cfg, &one_turn_spans());
    assert!(
        !path.exists(),
        "disabled tracing must not create the export file"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn export_appends_ndjson_to_the_configured_file() {
    let dir = std::env::temp_dir().join(format!("oh-trace-export-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("spans.ndjson");
    let cfg = AgentTracingConfig {
        enabled: true,
        backend: AgentTracingBackend::Otel,
        export_path: Some(path.to_string_lossy().to_string()),
        capture_content: false,
    };
    let spans = one_turn_spans();
    export_spans(&cfg, &spans);
    // Append, not truncate: a second export grows the file.
    export_spans(&cfg, &spans);

    let body = std::fs::read_to_string(&path).unwrap();
    assert_eq!(body.lines().count(), spans.len() * 2);
    // Each line is valid, parseable JSON.
    for line in body.lines() {
        let v: serde_json::Value = serde_json::from_str(line).unwrap();
        assert_eq!(v["trace_id"], serde_json::json!("sess-42"));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn export_with_no_path_does_not_panic() {
    // Path-less export logs instead of writing; must be safe to call.
    let cfg = AgentTracingConfig {
        enabled: true,
        backend: AgentTracingBackend::Langfuse,
        export_path: None,
        capture_content: false,
    };
    export_spans(&cfg, &one_turn_spans());
    export_spans(&cfg, &[]); // empty slice short-circuits.
}

#[tokio::test]
async fn export_run_trace_is_noop_when_disabled_or_empty() {
    // Both sharing AND the local exporter off → no-op regardless of spans.
    let mut disabled = crate::openhuman::config::Config::default();
    disabled.observability.share_usage_data = false;
    disabled.observability.agent_tracing.enabled = false;
    export_run_trace(&disabled, &one_turn_spans()).await;

    // No spans → no-op even with sharing on (the default).
    let enabled = crate::openhuman::config::Config::default();
    export_run_trace(&enabled, &[]).await;
}

#[tokio::test]
async fn export_run_trace_otel_backend_uses_local_sink() {
    // The Otel local exporter never touches the network — it writes the
    // file/log sink. Disable usage-data sharing to isolate that path (no push).
    let dir = std::env::temp_dir().join(format!("oh-trace-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("spans.ndjson");

    let mut config = crate::openhuman::config::Config::default();
    config.observability.share_usage_data = false;
    config.observability.agent_tracing = AgentTracingConfig {
        enabled: true,
        backend: AgentTracingBackend::Otel,
        export_path: Some(path.to_string_lossy().to_string()),
        capture_content: false,
    };
    export_run_trace(&config, &one_turn_spans()).await;

    let written = std::fs::read_to_string(&path).expect("otel export should write the file");
    assert!(
        !written.trim().is_empty(),
        "spans should be appended locally"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ── Route A: content + grouping + span-id uniqueness ────────────────────────

#[test]
fn turn_content_attaches_input_output_to_turn_span_when_capture_enabled() {
    // Content lands on the span ONLY when capture is opted in (#4454).
    let mut c = SpanCollector::new(ctx()).with_content_capture(true);
    c.record(&AgentProgress::TurnStarted, 1_000);
    c.record(
        &AgentProgress::TurnContent {
            input: Some("what is your favorite color?".to_string()),
            output: Some("i'm partial to teal".to_string()),
        },
        1_100,
    );
    let turn = find(c.spans(), "agent.turn");
    assert_eq!(
        turn.input.as_ref().and_then(|v| v.as_str()),
        Some("what is your favorite color?"),
        "TurnContent input must land on the turn span when capture_content=true"
    );
    assert_eq!(
        turn.output.as_ref().and_then(|v| v.as_str()),
        Some("i'm partial to teal")
    );
}

#[test]
fn turn_span_stamps_user_and_thread_grouping_attributes() {
    let mut c = SpanCollector::new(
        TraceContext::new("trace:req-1", Some("client-7".to_string()))
            .with_session_group("thread-abc"),
    );
    c.record(&AgentProgress::TurnStarted, 1_000);
    let turn = find(c.spans(), "agent.turn");
    assert_eq!(
        turn.attributes.get("user.id").and_then(|v| v.as_str()),
        Some("client-7")
    );
    assert_eq!(
        turn.attributes.get("thread.id").and_then(|v| v.as_str()),
        Some("thread-abc"),
        "session_group must be stamped as thread.id for the Langfuse sessionId"
    );
}

// ── identity / attribution / content capture ───────────────────────────────

#[test]
fn turn_span_carries_agent_client_and_source_attribution() {
    let mut c = SpanCollector::new(
        TraceContext::new("trace:req-9", Some("user-123".to_string()))
            .with_client_id("socket-abc")
            .with_agent_id("researcher")
            .with_channel_source("autonomous"),
    );
    c.record(&AgentProgress::TurnStarted, 0);
    // Trace name folds in the agent id.
    let turn = find(c.spans(), "agent.turn:researcher");
    assert_eq!(turn.kind, SpanKind::Turn);
    // Real user id is the user attribution; the transport client id is a
    // separate attribute, never conflated with the user.
    assert_eq!(turn.attributes["user.id"], serde_json::json!("user-123"));
    assert_eq!(
        turn.attributes["client.id"],
        serde_json::json!("socket-abc")
    );
    assert_eq!(turn.attributes["agent.id"], serde_json::json!("researcher"));
    assert_eq!(
        turn.attributes["channel.source"],
        serde_json::json!("autonomous")
    );
}

#[test]
fn turn_span_name_stays_plain_without_agent_id() {
    let mut c = SpanCollector::new(ctx());
    c.record(&AgentProgress::TurnStarted, 0);
    assert!(names(c.spans()).contains(&"agent.turn".to_string()));
}

#[test]
fn thread_id_falls_back_to_trace_id_without_session_group() {
    // Every trace must end up with a Langfuse sessionId: with no explicit
    // session group, the trace id itself is stamped as thread.id.
    let mut c = SpanCollector::new(TraceContext::new("sess-42:req-1", None));
    c.record(&AgentProgress::TurnStarted, 0);
    let turn = find(c.spans(), "agent.turn");
    assert_eq!(
        turn.attributes["thread.id"],
        serde_json::json!("sess-42:req-1")
    );
}

#[test]
fn tool_io_is_captured_when_capture_content_is_on() {
    let mut c = SpanCollector::new(ctx().with_capture_content(true));
    c.record(&AgentProgress::TurnStarted, 0);
    c.record(&tool_started("c1", "web_search", 1), 1);
    let tool = find(c.spans(), "tool.web_search");
    let input = tool.input.as_ref().and_then(|v| v.as_str()).unwrap();
    assert!(
        input.contains("do-not-export"),
        "tool arguments must be recorded as span input when capture is on"
    );

    // Subagent tool result → span output.
    c.record(&spawn("task-1", "Researcher"), 2);
    c.record(
        &AgentProgress::SubagentToolCallStarted {
            agent_id: "researcher".to_string(),
            task_id: "task-1".to_string(),
            call_id: "sc-1".to_string(),
            tool_name: "read_file".to_string(),
            arguments: serde_json::json!({"path": "notes.md"}),
            iteration: 1,
            display_label: None,
            display_detail: None,
        },
        3,
    );
    c.record(
        &AgentProgress::SubagentToolCallCompleted {
            agent_id: "researcher".to_string(),
            task_id: "task-1".to_string(),
            call_id: "sc-1".to_string(),
            tool_name: "read_file".to_string(),
            success: true,
            output_chars: 13,
            output: "file contents".to_string(),
            arguments: None,
            elapsed_ms: 4,
            iteration: 1,
            failure: None,
        },
        4,
    );
    let child_tool = find(c.spans(), "tool.read_file");
    assert!(child_tool
        .input
        .as_ref()
        .and_then(|v| v.as_str())
        .unwrap()
        .contains("notes.md"));
    assert_eq!(
        child_tool.output.as_ref().and_then(|v| v.as_str()),
        Some("file contents")
    );
}

#[test]
fn tool_io_is_never_recorded_when_capture_content_is_off() {
    // Default ctx() has capture_content = false.
    let mut c = collect(&[
        (AgentProgress::TurnStarted, 0),
        (tool_started("c1", "web_search", 1), 1),
        (spawn("task-1", "Researcher"), 2),
        (
            AgentProgress::SubagentToolCallStarted {
                agent_id: "researcher".to_string(),
                task_id: "task-1".to_string(),
                call_id: "sc-1".to_string(),
                tool_name: "read_file".to_string(),
                arguments: serde_json::json!({"path": "secret.md"}),
                iteration: 1,
                display_label: None,
                display_detail: None,
            },
            3,
        ),
        (
            AgentProgress::SubagentToolCallCompleted {
                agent_id: "researcher".to_string(),
                task_id: "task-1".to_string(),
                call_id: "sc-1".to_string(),
                tool_name: "read_file".to_string(),
                success: true,
                output_chars: 6,
                output: "sekrit".to_string(),
                arguments: None,
                elapsed_ms: 4,
                iteration: 1,
                failure: None,
            },
            4,
        ),
    ]);
    c.finish(100);
    for span in c.spans() {
        if span.kind == SpanKind::Tool {
            assert!(span.input.is_none(), "no tool input with capture off");
            assert!(span.output.is_none(), "no tool output with capture off");
        }
    }
    let blob = serde_json::to_string(c.spans()).unwrap();
    assert!(!blob.contains("do-not-export"));
    assert!(!blob.contains("sekrit"));
}

#[test]
fn captured_tool_io_is_truncated_with_marker() {
    let mut c = SpanCollector::new(ctx().with_capture_content(true));
    c.record(&AgentProgress::TurnStarted, 0);
    let huge = "x".repeat(10_000);
    c.record(
        &AgentProgress::ToolCallStarted {
            call_id: "c1".to_string(),
            tool_name: "shell".to_string(),
            arguments: serde_json::json!({ "cmd": huge }),
            iteration: 1,
            display_label: None,
            display_detail: None,
        },
        1,
    );
    let tool = find(c.spans(), "tool.shell");
    let input = tool.input.as_ref().and_then(|v| v.as_str()).unwrap();
    assert!(input.contains("[truncated"), "marker must flag truncation");
    assert!(
        input.chars().count() < 4_100,
        "captured input must be capped near 4000 chars, got {}",
        input.chars().count()
    );
}

// ── per-call generations + provenance + reasoning/cache-write usage ─────────

fn model_call(model: &str, reasoning: u64, cache_write: u64) -> AgentProgress {
    AgentProgress::ModelCallCompleted {
        model: model.to_string(),
        provider_id: "managed".to_string(),
        subagent_task_id: None,
        input: None,
        output: None,
        iteration: 1,
        input_tokens: 1_000,
        output_tokens: 200,
        cached_input_tokens: 300,
        cache_creation_tokens: cache_write,
        reasoning_tokens: reasoning,
        cost_usd: 0.0042,
    }
}

#[test]
fn model_call_completed_emits_generation_span_with_usage_cost_and_pricing() {
    let mut c = collect(&[
        (AgentProgress::TurnStarted, 1_000),
        (
            AgentProgress::IterationStarted {
                iteration: 1,
                max_iterations: 5,
            },
            1_010,
        ),
        (model_call("agentic-v1", 0, 0), 1_500),
    ]);
    c.finish(2_000);
    let spans = c.spans();

    let generation = find(spans, "llm.agentic-v1");
    assert_eq!(generation.kind, SpanKind::Generation);
    // Parented under the live iteration; starts at the iteration start
    // (ModelStarted) and ends when the usage record was observed.
    let iter = find(spans, "agent.iteration#1");
    assert_eq!(
        generation.parent_span_id.as_deref(),
        Some(iter.span_id.as_str())
    );
    assert_eq!(generation.start_unix_ms, 1_010);
    assert_eq!(generation.end_unix_ms, Some(1_500));
    assert_eq!(generation.status, SpanStatus::Ok);

    let a = &generation.attributes;
    // Provider-labeled model: `{provider_id}.{model}`.
    assert_eq!(
        a["gen_ai.request.model"],
        serde_json::json!("managed.agentic-v1")
    );
    assert_eq!(a["gen_ai.usage.input_tokens"], serde_json::json!(1_000));
    assert_eq!(a["gen_ai.usage.output_tokens"], serde_json::json!(200));
    // Cache reads always flow, even when other calls happen to be zero.
    assert_eq!(
        a["gen_ai.usage.cached_input_tokens"],
        serde_json::json!(300)
    );
    assert_eq!(a["gen_ai.usage.cost_usd"], serde_json::json!(0.0042));
    // Managed tier handle → managed provenance.
    assert_eq!(a["gen_ai.provider"], serde_json::json!("managed"));
    // Pricing basis is auditable.
    assert_eq!(
        a["gen_ai.pricing.input_per_mtok_usd"],
        serde_json::json!(0.435)
    );
    assert!(a.get("gen_ai.pricing.output_per_mtok_usd").is_some());
    // Zero reasoning / cache-write tokens are omitted on the generation.
    assert!(a.get("gen_ai.usage.reasoning_tokens").is_none());
    assert!(a.get("gen_ai.usage.cache_creation_tokens").is_none());
}

#[test]
fn custom_model_generation_is_stamped_custom_provenance() {
    // A BYO model rides whatever provider id the event carries ("openai",
    // "ollama", or the "custom" default) — both on the generation and the root.
    let event = AgentProgress::ModelCallCompleted {
        model: "claude-imaginary-9".to_string(),
        provider_id: "custom".to_string(),
        subagent_task_id: None,
        input: None,
        output: None,
        iteration: 1,
        input_tokens: 10,
        output_tokens: 2,
        cached_input_tokens: 0,
        cache_creation_tokens: 0,
        reasoning_tokens: 0,
        cost_usd: 0.0001,
    };
    let mut c = collect(&[(AgentProgress::TurnStarted, 0), (event, 10)]);
    c.finish(20);
    let generation = find(c.spans(), "llm.claude-imaginary-9");
    assert_eq!(
        generation.attributes["gen_ai.provider"],
        serde_json::json!("custom")
    );
    assert_eq!(
        generation.attributes["gen_ai.request.model"],
        serde_json::json!("custom.claude-imaginary-9")
    );
    // Provenance also lands on the root turn span (→ trace metadata).
    let turn = find(c.spans(), "agent.turn");
    assert_eq!(
        turn.attributes["gen_ai.provider"],
        serde_json::json!("custom")
    );
}

#[test]
fn reasoning_and_cache_write_tokens_flow_to_generation_and_root_rollup() {
    let mut c = collect(&[
        (AgentProgress::TurnStarted, 0),
        (model_call("agentic-v1", 128, 64), 10),
        (model_call("agentic-v1", 72, 0), 20),
    ]);
    c.finish(30);
    let spans = c.spans();

    // Per-call values on the generations.
    let generations: Vec<&TraceSpan> = spans
        .iter()
        .filter(|s| s.kind == SpanKind::Generation)
        .collect();
    assert_eq!(generations.len(), 2, "one generation per model call");
    assert_eq!(
        generations[0].attributes["gen_ai.usage.reasoning_tokens"],
        serde_json::json!(128)
    );
    assert_eq!(
        generations[0].attributes["gen_ai.usage.cache_creation_tokens"],
        serde_json::json!(64)
    );
    assert_eq!(
        generations[1].attributes["gen_ai.usage.reasoning_tokens"],
        serde_json::json!(72)
    );

    // Cumulative rollup on the root turn span (TurnCostUpdated doesn't carry
    // these dimensions).
    let turn = find(spans, "agent.turn");
    assert_eq!(
        turn.attributes["gen_ai.usage.reasoning_tokens"],
        serde_json::json!(200)
    );
    assert_eq!(
        turn.attributes["gen_ai.usage.cache_creation_tokens"],
        serde_json::json!(64)
    );
}

#[test]
fn zero_reasoning_turn_leaves_root_without_reasoning_attr() {
    let mut c = collect(&[
        (AgentProgress::TurnStarted, 0),
        (model_call("agentic-v1", 0, 0), 10),
    ]);
    c.finish(20);
    let turn = find(c.spans(), "agent.turn");
    assert!(turn
        .attributes
        .get("gen_ai.usage.reasoning_tokens")
        .is_none());
    assert!(turn
        .attributes
        .get("gen_ai.usage.cache_creation_tokens")
        .is_none());
}

// ── run-type classification ─────────────────────────────────────────────────

#[test]
fn run_type_classifies_known_sources() {
    assert_eq!(RunType::from_source(None), RunType::InteractiveChat);
    assert_eq!(RunType::from_source(Some("ptt")), RunType::InteractiveChat);
    assert_eq!(RunType::from_source(Some("type")), RunType::InteractiveChat);
    assert_eq!(
        RunType::from_source(Some("autonomous")),
        RunType::AutonomousTask
    );
    assert_eq!(RunType::from_source(Some("agentbox")), RunType::Agentbox);
    assert_eq!(
        RunType::from_source(Some("channel_inbound")),
        RunType::ChannelInbound
    );
    assert_eq!(RunType::AutonomousTask.as_str(), "autonomous_task");
}

#[test]
fn run_type_is_stamped_on_the_turn_span() {
    // Default: interactive chat.
    let mut c = SpanCollector::new(ctx());
    c.record(&AgentProgress::TurnStarted, 0);
    assert_eq!(
        find(c.spans(), "agent.turn").attributes["run.type"],
        serde_json::json!("interactive_chat")
    );

    // Explicit autonomous run.
    let mut c = SpanCollector::new(ctx().with_run_type(RunType::AutonomousTask));
    c.record(&AgentProgress::TurnStarted, 0);
    assert_eq!(
        find(c.spans(), "agent.turn").attributes["run.type"],
        serde_json::json!("autonomous_task")
    );
}

// ── error text capture (Langfuse statusMessage source) ─────────────────────

#[test]
fn subagent_failure_records_truncated_error_message_when_capture_on() {
    let mut c = SpanCollector::new(ctx().with_capture_content(true));
    c.record(&AgentProgress::TurnStarted, 0);
    c.record(&spawn("task-9", "Coder"), 5);
    let long_error = "boom ".repeat(200); // 1000 chars > 500 cap
    c.record(
        &AgentProgress::SubagentFailed {
            agent_id: "coder".to_string(),
            task_id: "task-9".to_string(),
            error: long_error,
        },
        10,
    );
    let sub = find(c.spans(), "subagent.Coder");
    assert_eq!(sub.status, SpanStatus::Error);
    let message = sub.attributes["error.message"].as_str().unwrap();
    assert!(message.starts_with("boom "));
    assert!(message.contains("[truncated"), "500-char cap must apply");
    assert!(message.chars().count() < 600);
}

#[test]
fn failed_tool_records_classified_cause_only_when_capture_on() {
    use crate::openhuman::tool_status::{ClassifiedFailure, FailureCategory, ToolFailureClass};
    let failed = AgentProgress::ToolCallCompleted {
        call_id: "c1".to_string(),
        tool_name: "shell".to_string(),
        success: false,
        output_chars: 0,
        output: String::new(),
        arguments: None,
        elapsed_ms: 5,
        iteration: 1,
        failure: Some(ClassifiedFailure {
            class: ToolFailureClass::Timeout,
            category: FailureCategory::Recoverable,
            cause_plain: "The command ran past its deadline".to_string(),
            next_action: "Try again".to_string(),
            recoverable: true,
        }),
    };

    // Capture ON → plain-language cause lands as error.message.
    let mut on = SpanCollector::new(ctx().with_capture_content(true));
    on.record(&AgentProgress::TurnStarted, 0);
    on.record(&tool_started("c1", "shell", 1), 1);
    on.record(&failed, 2);
    let tool = find(on.spans(), "tool.shell");
    assert_eq!(tool.status, SpanStatus::Error);
    assert_eq!(
        tool.attributes["error.message"],
        serde_json::json!("The command ran past its deadline")
    );

    // Capture OFF → no error text on the span.
    let mut off = SpanCollector::new(ctx());
    off.record(&AgentProgress::TurnStarted, 0);
    off.record(&tool_started("c1", "shell", 1), 1);
    off.record(&failed, 2);
    let tool = find(off.spans(), "tool.shell");
    assert_eq!(tool.status, SpanStatus::Error);
    assert!(tool.attributes.get("error.message").is_none());
}

#[test]
fn subagent_error_text_stays_out_without_capture() {
    // The pre-existing privacy behavior (length-only) holds with capture off —
    // covered by `subagent_failure_records_error_without_raw_text` above; here
    // we double-check error.message is absent.
    let mut c = collect(&[
        (AgentProgress::TurnStarted, 0),
        (spawn("task-9", "Coder"), 5),
        (
            AgentProgress::SubagentFailed {
                agent_id: "coder".to_string(),
                task_id: "task-9".to_string(),
                error: "secret path /Users/x".to_string(),
            },
            10,
        ),
    ]);
    c.finish(50);
    let sub = find(c.spans(), "subagent.Coder");
    assert!(sub.attributes.get("error.message").is_none());
}

#[test]
fn span_ids_are_unique_across_turns() {
    // Two separate collectors (two turns) must not reuse span ids, or Langfuse
    // dedupes their observations onto whichever trace claimed the id first.
    let a = collect(&[(AgentProgress::TurnStarted, 1)]);
    let b = collect(&[(AgentProgress::TurnStarted, 1)]);
    assert_ne!(
        find(a.spans(), "agent.turn").span_id,
        find(b.spans(), "agent.turn").span_id,
        "span ids must be globally unique across turns"
    );
}

// ── content-bearing wiring (system prompt / tool IO / subagent IO) ──────────

fn capture_ctx() -> TraceContext {
    ctx().with_capture_content(true)
}

fn collect_with_capture(events: &[(AgentProgress, u64)]) -> SpanCollector {
    let mut c = SpanCollector::new(capture_ctx());
    for (event, ts) in events {
        c.record(event, *ts);
    }
    c
}

fn model_call_with_content(subagent_task_id: Option<&str>) -> AgentProgress {
    AgentProgress::ModelCallCompleted {
        model: "chat-v1".to_string(),
        provider_id: "managed".to_string(),
        subagent_task_id: subagent_task_id.map(str::to_string),
        input: Some(serde_json::json!([
            {"role": "system", "content": "You are OpenHuman."},
            {"role": "user", "content": "hi"}
        ])),
        output: Some(serde_json::json!({"role": "assistant", "content": "hello"})),
        iteration: 1,
        input_tokens: 100,
        output_tokens: 10,
        cached_input_tokens: 0,
        cache_creation_tokens: 0,
        reasoning_tokens: 0,
        cost_usd: 0.001,
    }
}

#[test]
fn generation_records_request_messages_and_completion_when_capture_on() {
    let mut c = collect_with_capture(&[
        (AgentProgress::TurnStarted, 0),
        (model_call_with_content(None), 10),
    ]);
    c.finish(20);
    let generation = find(c.spans(), "llm.chat-v1");
    let input = generation.input.as_ref().expect("generation input");
    assert!(
        input.to_string().contains("You are OpenHuman."),
        "system prompt must land in the generation input: {input}"
    );
    assert!(generation
        .output
        .as_ref()
        .expect("generation output")
        .to_string()
        .contains("hello"));
}

#[test]
fn generation_withholds_content_when_capture_off() {
    let mut c = collect(&[
        (AgentProgress::TurnStarted, 0),
        (model_call_with_content(None), 10),
    ]);
    c.finish(20);
    let generation = find(c.spans(), "llm.chat-v1");
    assert!(generation.input.is_none(), "capture off → no prompt");
    assert!(generation.output.is_none(), "capture off → no completion");
}

#[test]
fn subagent_model_call_nests_generation_and_stamps_model_on_subagent_span() {
    let mut c = collect_with_capture(&[
        (AgentProgress::TurnStarted, 0),
        (spawn("task-9", "Context Scout"), 5),
        (
            AgentProgress::SubagentIterationStarted {
                agent_id: "context_scout".to_string(),
                task_id: "task-9".to_string(),
                iteration: 1,
                max_iterations: 8,
                extended_policy: true,
            },
            6,
        ),
        (model_call_with_content(Some("task-9")), 10),
    ]);
    c.finish(20);
    let spans = c.spans();

    // Generation nests under the subagent's iteration, not the parent turn.
    let generation = find(spans, "llm.chat-v1");
    let child_iter = find(spans, "subagent.iteration#1");
    assert_eq!(
        generation.parent_span_id.as_deref(),
        Some(child_iter.span_id.as_str())
    );
    assert!(generation.input.is_some(), "child generation carries input");

    // The subagent span itself surfaces the provider-labeled model + usage.
    let sub = find(spans, "subagent.Context Scout");
    assert_eq!(
        sub.attributes["gen_ai.request.model"],
        serde_json::json!("managed.chat-v1")
    );
    assert_eq!(
        sub.attributes["gen_ai.usage.input_tokens"],
        serde_json::json!(100)
    );
    assert_eq!(
        sub.attributes["gen_ai.usage.cost_usd"],
        serde_json::json!(0.001)
    );

    // The parent turn's rollup is NOT polluted by the child call.
    let turn = find(spans, "agent.turn");
    assert!(turn.attributes.get("gen_ai.request.model").is_none());
}

#[test]
fn parent_tool_completion_backfills_arguments_and_records_output() {
    let mut c = collect_with_capture(&[
        (AgentProgress::TurnStarted, 0),
        (
            // tinyagents path: Started carries Null arguments.
            AgentProgress::ToolCallStarted {
                call_id: "c1".to_string(),
                tool_name: "web_search".to_string(),
                arguments: serde_json::Value::Null,
                iteration: 1,
                display_label: None,
                display_detail: None,
            },
            5,
        ),
        (
            AgentProgress::ToolCallCompleted {
                call_id: "c1".to_string(),
                tool_name: "web_search".to_string(),
                success: true,
                output_chars: 7,
                output: "results".to_string(),
                arguments: Some(serde_json::json!({"query": "weather"})),
                elapsed_ms: 40,
                iteration: 1,
                failure: None,
            },
            45,
        ),
    ]);
    c.finish(50);
    let tool = find(c.spans(), "tool.web_search");
    assert!(
        tool.input
            .as_ref()
            .expect("tool input backfilled from completion")
            .to_string()
            .contains("weather"),
        "arguments from the completion event must backfill the span input"
    );
    assert_eq!(
        tool.output,
        Some(serde_json::Value::String("results".to_string()))
    );
}

#[test]
fn subagent_span_records_prompt_and_final_output_when_capture_on() {
    let mut c = collect_with_capture(&[
        (AgentProgress::TurnStarted, 0),
        (spawn("task-1", "Researcher"), 5),
        (
            AgentProgress::SubagentCompleted {
                agent_id: "researcher".to_string(),
                task_id: "task-1".to_string(),
                elapsed_ms: 100,
                iterations: 2,
                output_chars: 12,
                output: "final answer".to_string(),
                worktree_path: None,
                changed_files: vec![],
                dirty_status: None,
            },
            105,
        ),
    ]);
    c.finish(110);
    let sub = find(c.spans(), "subagent.Researcher");
    assert_eq!(
        sub.input,
        Some(serde_json::Value::String("delegated prompt".to_string()))
    );
    assert_eq!(
        sub.output,
        Some(serde_json::Value::String("final answer".to_string()))
    );
}

#[test]
fn subagent_content_is_withheld_when_capture_off() {
    let mut c = collect(&[
        (AgentProgress::TurnStarted, 0),
        (spawn("task-1", "Researcher"), 5),
        (
            AgentProgress::SubagentCompleted {
                agent_id: "researcher".to_string(),
                task_id: "task-1".to_string(),
                elapsed_ms: 100,
                iterations: 2,
                output_chars: 12,
                output: "final answer".to_string(),
                worktree_path: None,
                changed_files: vec![],
                dirty_status: None,
            },
            105,
        ),
    ]);
    c.finish(110);
    let sub = find(c.spans(), "subagent.Researcher");
    assert!(sub.input.is_none());
    assert!(sub.output.is_none());
}

#[test]
fn oversized_model_content_degrades_to_truncated_string() {
    let big = "x".repeat(MAX_MODEL_CONTENT_CHARS + 100);
    let captured = capture_model_content(&serde_json::json!({ "content": big }));
    let rendered = match &captured {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    assert!(rendered.chars().count() <= MAX_MODEL_CONTENT_CHARS + 64);
    assert!(rendered.contains("truncated"));
}

#[test]
fn turn_content_respects_the_trace_context_capture_gate() {
    // Regression (PR #4506 review): the collector briefly carried TWO capture
    // gates — a collector-level flag (checked by the TurnContent arm) and the
    // TraceContext flag (checked everywhere else). The web progress bridge only
    // sets the TraceContext flag, so TurnContent silently dropped the turn's
    // prompt/reply even with capture_content enabled. There is now a single
    // gate: both construction styles must attach TurnContent.
    for collector in [
        SpanCollector::new(ctx().with_capture_content(true)),
        SpanCollector::new(ctx()).with_content_capture(true),
    ] {
        let mut c = collector;
        c.record(&AgentProgress::TurnStarted, 0);
        c.record(
            &AgentProgress::TurnContent {
                input: Some("the prompt".to_string()),
                output: Some("the reply".to_string()),
            },
            5,
        );
        c.finish(10);
        let turn = find(c.spans(), "agent.turn");
        assert_eq!(
            turn.input,
            Some(serde_json::Value::String("the prompt".to_string()))
        );
        assert_eq!(
            turn.output,
            Some(serde_json::Value::String("the reply".to_string()))
        );
    }
}
