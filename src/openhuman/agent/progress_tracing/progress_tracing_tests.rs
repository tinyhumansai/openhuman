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

// ── serialization + export ──────────────────────────────────────────────────

fn one_turn_spans() -> Vec<TraceSpan> {
    let mut c = collect(&[
        (AgentProgress::TurnStarted, 0),
        (AgentProgress::TurnCompleted { iterations: 1 }, 10),
    ]);
    c.finish(10);
    c.into_spans()
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

#[path = "progress_tracing_tests_part_01_tests.rs"]
mod part_01_tests;
#[path = "progress_tracing_tests_part_02_tests.rs"]
mod part_02_tests;
#[path = "progress_tracing_tests_part_03_tests.rs"]
mod part_03_tests;
