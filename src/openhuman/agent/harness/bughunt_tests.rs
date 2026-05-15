//! Targeted bug-hunt tests for the agent harness + tool dispatch.
//!
//! These pair the [`super::test_support::KeywordScriptedProvider`] with
//! tightly-scoped tools to probe corner cases that aren't covered by
//! the broader behavioural suite. Each test documents the behaviour
//! observed, and any tests prefixed `documents_` describe a quirk
//! worth flagging in code review (silent data loss, surprising
//! precedence rules, etc.) rather than asserting correctness.

use super::test_support::{KeywordRule, KeywordScriptedProvider, ScriptedToolCall};
use super::tool_loop::run_tool_call_loop;
use crate::openhuman::providers::{ChatMessage, ChatResponse, ToolCall};
use crate::openhuman::tools::traits::{Tool, ToolResult};
use async_trait::async_trait;
use parking_lot::Mutex;
use serde_json::json;
use std::sync::Arc;

fn mm() -> crate::openhuman::config::MultimodalConfig {
    crate::openhuman::config::MultimodalConfig::default()
}

struct ArgsCapturingTool {
    name_str: String,
    captured: Arc<Mutex<Vec<serde_json::Value>>>,
    output: String,
}

impl ArgsCapturingTool {
    fn new(name: &str, output: &str) -> (Self, Arc<Mutex<Vec<serde_json::Value>>>) {
        let captured = Arc::new(Mutex::new(Vec::new()));
        (
            Self {
                name_str: name.to_string(),
                captured: captured.clone(),
                output: output.to_string(),
            },
            captured,
        )
    }
}

#[async_trait]
impl Tool for ArgsCapturingTool {
    fn name(&self) -> &str {
        &self.name_str
    }
    fn description(&self) -> &str {
        "captures args"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({"type":"object","additionalProperties":true})
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        self.captured.lock().push(args);
        Ok(ToolResult::success(self.output.clone()))
    }
}

// ── 1. Native tool call with a JSON-encoded string of args ────────
//
// Real OpenAI/Anthropic providers send `arguments` as a *string*
// containing JSON. The harness must transparently decode it before
// passing to the tool.

#[tokio::test]
async fn native_tool_call_decodes_json_encoded_arguments_string() {
    let provider =
        KeywordScriptedProvider::new(vec![KeywordRule::final_reply("captured-ok", "done")])
            .with_native_tools(true);

    // Forced first turn: native tool_call with arguments as a STRING.
    provider.push_forced_response(ChatResponse {
        text: None,
        tool_calls: vec![ToolCall {
            id: "c1".into(),
            name: "captured".into(),
            arguments: "{\"city\":\"Berlin\",\"n\":3}".to_string(),
        }],
        usage: None,
    });

    let (tool, captured) = ArgsCapturingTool::new("captured", "captured-ok");
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(tool)];
    let mut history = vec![ChatMessage::user("anything")];

    let out = run_tool_call_loop(
        &provider,
        &mut history,
        &tools,
        "mock",
        "m",
        0.0,
        true,
        None,
        "channel",
        &mm(),
        3,
        None,
        None,
        &[],
        None,
        None,
    )
    .await
    .unwrap();

    assert_eq!(out, "done");
    let args = captured.lock();
    assert_eq!(args.len(), 1);
    assert_eq!(args[0]["city"], "Berlin");
    assert_eq!(args[0]["n"], 3);
}

// ── 2. SILENT FAILURE: non-JSON args string is replaced with `{}` ──
//
// `parse_arguments_value` calls `serde_json::from_str` on the string
// payload and silently falls back to `{}` on parse failure. This
// means: if a model emits `arguments: "world"` (not valid JSON), the
// tool sees `{}` — the user's intent is silently dropped and there's
// no signal to the LLM that anything went wrong.
//
// This test documents the behaviour so future refactors don't
// "accidentally" fix it without considering downstream impact, and
// flags the behaviour for follow-up.

#[tokio::test]
async fn documents_silent_drop_of_non_json_arguments_string() {
    let provider =
        KeywordScriptedProvider::new(vec![KeywordRule::final_reply("captured-ok", "done")])
            .with_native_tools(true);

    provider.push_forced_response(ChatResponse {
        text: None,
        tool_calls: vec![ToolCall {
            id: "c1".into(),
            name: "captured".into(),
            // Not valid JSON — the model "meant" a plain string.
            arguments: "world".to_string(),
        }],
        usage: None,
    });

    let (tool, captured) = ArgsCapturingTool::new("captured", "captured-ok");
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(tool)];
    let mut history = vec![ChatMessage::user("hi")];

    run_tool_call_loop(
        &provider,
        &mut history,
        &tools,
        "mock",
        "m",
        0.0,
        true,
        None,
        "channel",
        &mm(),
        3,
        None,
        None,
        &[],
        None,
        None,
    )
    .await
    .unwrap();

    let args = captured.lock();
    assert_eq!(args.len(), 1);
    // BUG-CANDIDATE: the LLM's intent ("world") is silently dropped.
    // The tool receives an empty object with no indication the args
    // were unparseable. A more defensive design would surface a
    // structured error back to the model instead.
    assert_eq!(args[0], json!({}));
}

// ── 3. Parallel tool calls in a single iteration ──────────────────
//
// The model may emit multiple `<tool_call>` blocks at once. They
// should all execute in order, each result threaded into history.

#[tokio::test]
async fn parallel_tool_calls_in_single_iteration_all_execute() {
    let provider =
        KeywordScriptedProvider::new(vec![KeywordRule::final_reply("tool_b-ok", "all done")]);

    // Both tool calls share one assistant turn (XML path).
    provider.push_forced_response(ChatResponse {
        text: Some(
            "<tool_call>{\"name\":\"tool_a\",\"arguments\":{\"k\":1}}</tool_call>\n\
             <tool_call>{\"name\":\"tool_b\",\"arguments\":{\"k\":2}}</tool_call>"
                .into(),
        ),
        tool_calls: vec![],
        usage: None,
    });

    let (a, a_calls) = ArgsCapturingTool::new("tool_a", "tool_a-ok");
    let (b, b_calls) = ArgsCapturingTool::new("tool_b", "tool_b-ok");
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(a), Box::new(b)];
    let mut history = vec![ChatMessage::user("do both")];

    let out = run_tool_call_loop(
        &provider,
        &mut history,
        &tools,
        "mock",
        "m",
        0.0,
        true,
        None,
        "channel",
        &mm(),
        5,
        None,
        None,
        &[],
        None,
        None,
    )
    .await
    .unwrap();

    assert_eq!(out, "all done");
    assert_eq!(a_calls.lock().len(), 1);
    assert_eq!(b_calls.lock().len(), 1);
    assert_eq!(a_calls.lock()[0]["k"], 1);
    assert_eq!(b_calls.lock()[0]["k"], 2);
}

// ── 4. Same-named tools: first match in registry wins ─────────────

#[tokio::test]
async fn same_named_tool_in_registry_first_match_wins() {
    let provider = KeywordScriptedProvider::new(vec![
        KeywordRule::tool_call("go", ScriptedToolCall::new("dupe", json!({}))),
        KeywordRule::final_reply("first-output", "got first"),
    ]);

    let (first, first_calls) = ArgsCapturingTool::new("dupe", "first-output");
    let (second, second_calls) = ArgsCapturingTool::new("dupe", "second-output");
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(first), Box::new(second)];
    let mut history = vec![ChatMessage::user("go ahead")];

    let out = run_tool_call_loop(
        &provider,
        &mut history,
        &tools,
        "mock",
        "m",
        0.0,
        true,
        None,
        "channel",
        &mm(),
        5,
        None,
        None,
        &[],
        None,
        None,
    )
    .await
    .unwrap();

    assert_eq!(out, "got first");
    assert_eq!(first_calls.lock().len(), 1);
    assert_eq!(second_calls.lock().len(), 0);
}

// ── 5. Markdown-fenced tool call (```tool_call ... ```) ───────────
//
// Some OpenRouter-mediated models emit fenced markdown blocks
// instead of XML tags. `parse_tool_calls` is supposed to handle this.

#[tokio::test]
async fn markdown_fenced_tool_call_block_is_parsed() {
    let provider =
        KeywordScriptedProvider::new(vec![KeywordRule::final_reply("tool_a-ok", "ok done")]);

    provider.push_forced_response(ChatResponse {
        text: Some(
            "Here's the call:\n\
             ```tool_call\n\
             {\"name\":\"tool_a\",\"arguments\":{\"x\":42}}\n\
             ```"
            .into(),
        ),
        tool_calls: vec![],
        usage: None,
    });

    let (a, a_calls) = ArgsCapturingTool::new("tool_a", "tool_a-ok");
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(a)];
    let mut history = vec![ChatMessage::user("anything")];

    let out = run_tool_call_loop(
        &provider,
        &mut history,
        &tools,
        "mock",
        "m",
        0.0,
        true,
        None,
        "channel",
        &mm(),
        5,
        None,
        None,
        &[],
        None,
        None,
    )
    .await
    .unwrap();

    assert_eq!(out, "ok done");
    assert_eq!(a_calls.lock().len(), 1);
    assert_eq!(a_calls.lock()[0]["x"], 42);
}

// ── 6. Native vs prompt-guided precedence ─────────────────────────
//
// When a response carries BOTH native `tool_calls` *and* an XML
// `<tool_call>` block in the text, the native calls are authoritative
// and the XML must NOT also fire (else the same logical call could
// execute twice).

#[tokio::test]
async fn native_tool_calls_take_precedence_over_xml_in_text() {
    let provider =
        KeywordScriptedProvider::new(vec![KeywordRule::final_reply("tool_a-ok", "done")])
            .with_native_tools(true);

    provider.push_forced_response(ChatResponse {
        text: Some("<tool_call>{\"name\":\"tool_a\",\"arguments\":{}}</tool_call>".into()),
        tool_calls: vec![ToolCall {
            id: "c1".into(),
            name: "tool_a".into(),
            arguments: "{\"src\":\"native\"}".into(),
        }],
        usage: None,
    });

    let (a, a_calls) = ArgsCapturingTool::new("tool_a", "tool_a-ok");
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(a)];
    let mut history = vec![ChatMessage::user("call it")];

    run_tool_call_loop(
        &provider,
        &mut history,
        &tools,
        "mock",
        "m",
        0.0,
        true,
        None,
        "channel",
        &mm(),
        5,
        None,
        None,
        &[],
        None,
        None,
    )
    .await
    .unwrap();

    // Tool ran exactly once, using the native args (not the XML block).
    assert_eq!(a_calls.lock().len(), 1);
    assert_eq!(a_calls.lock()[0]["src"], "native");
}

// ── 7. Big tool output: per-tool cap truncation ───────────────────

struct CappedBigTool;

#[async_trait]
impl Tool for CappedBigTool {
    fn name(&self) -> &str {
        "cap_big"
    }
    fn description(&self) -> &str {
        "emits a big payload but caps it"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({"type":"object"})
    }
    fn max_result_size_chars(&self) -> Option<usize> {
        Some(50)
    }
    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        Ok(ToolResult::success("X".repeat(500)))
    }
}

#[tokio::test]
async fn per_tool_max_result_size_caps_history_payload() {
    let provider = KeywordScriptedProvider::new(vec![
        KeywordRule::tool_call("go", ScriptedToolCall::new("cap_big", json!({}))),
        KeywordRule::final_reply("truncated by tool cap", "ok"),
    ]);

    let tools: Vec<Box<dyn Tool>> = vec![Box::new(CappedBigTool)];
    let mut history = vec![ChatMessage::user("go big")];

    run_tool_call_loop(
        &provider,
        &mut history,
        &tools,
        "mock",
        "m",
        0.0,
        true,
        None,
        "channel",
        &mm(),
        5,
        None,
        None,
        &[],
        None,
        None,
    )
    .await
    .unwrap();

    let tool_results = history
        .iter()
        .find(|msg| msg.role == "user" && msg.content.contains("[Tool results]"))
        .expect("tool results should land in history");
    assert!(
        tool_results.content.contains("truncated by tool cap"),
        "cap marker missing: {}",
        tool_results.content
    );
    assert!(
        tool_results.content.len() < 500,
        "raw 500-char body must not flow through (got {} chars)",
        tool_results.content.len()
    );
}

// ── 8. Empty assistant response with no tool calls terminates loop

#[tokio::test]
async fn empty_response_with_no_tool_calls_terminates_with_empty_text() {
    let provider = KeywordScriptedProvider::new(vec![]);
    provider.push_forced_response(ChatResponse {
        text: Some(String::new()),
        tool_calls: vec![],
        usage: None,
    });

    let tools: Vec<Box<dyn Tool>> = vec![];
    let mut history = vec![ChatMessage::user("hi")];

    let out = run_tool_call_loop(
        &provider,
        &mut history,
        &tools,
        "mock",
        "m",
        0.0,
        true,
        None,
        "channel",
        &mm(),
        5,
        None,
        None,
        &[],
        None,
        None,
    )
    .await
    .unwrap();

    assert!(out.is_empty());
    // Loop must still record the (empty) assistant turn.
    assert!(history.iter().any(|m| m.role == "assistant"));
}

// ── 9. Progress sink receives ordered turn lifecycle events ───────

#[tokio::test]
async fn progress_sink_emits_lifecycle_events_in_order() {
    use crate::openhuman::agent::progress::AgentProgress;

    let provider = KeywordScriptedProvider::new(vec![
        KeywordRule::tool_call("go", ScriptedToolCall::new("p_tool", json!({}))),
        KeywordRule::final_reply("p_tool-ok", "all done"),
    ]);

    let (tool, _) = ArgsCapturingTool::new("p_tool", "p_tool-ok");
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(tool)];

    let (tx, mut rx) = tokio::sync::mpsc::channel::<AgentProgress>(32);

    let mut history = vec![ChatMessage::user("go go")];
    run_tool_call_loop(
        &provider,
        &mut history,
        &tools,
        "mock",
        "m",
        0.0,
        true,
        None,
        "channel",
        &mm(),
        5,
        None,
        None,
        &[],
        Some(tx),
        None,
    )
    .await
    .unwrap();

    let mut events = Vec::new();
    while let Ok(e) = rx.try_recv() {
        events.push(e);
    }

    // Must see TurnStarted, at least 2 IterationStarted, ToolCallStarted,
    // ToolCallCompleted, and TurnCompleted.
    let kinds: Vec<&'static str> = events
        .iter()
        .map(|e| match e {
            AgentProgress::TurnStarted => "TurnStarted",
            AgentProgress::IterationStarted { .. } => "IterationStarted",
            AgentProgress::ToolCallStarted { .. } => "ToolCallStarted",
            AgentProgress::ToolCallCompleted { .. } => "ToolCallCompleted",
            AgentProgress::TurnCompleted { .. } => "TurnCompleted",
            _ => "Other",
        })
        .collect();

    assert_eq!(kinds.first().copied(), Some("TurnStarted"));
    assert_eq!(kinds.last().copied(), Some("TurnCompleted"));
    assert!(kinds.contains(&"IterationStarted"));
    assert!(kinds.contains(&"ToolCallStarted"));
    assert!(kinds.contains(&"ToolCallCompleted"));

    // ToolCallStarted must precede its matching ToolCallCompleted.
    let started = kinds.iter().position(|k| *k == "ToolCallStarted").unwrap();
    let completed = kinds
        .iter()
        .position(|k| *k == "ToolCallCompleted")
        .unwrap();
    assert!(started < completed);
}
