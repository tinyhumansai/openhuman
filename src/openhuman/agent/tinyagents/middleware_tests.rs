use super::*;
use serde_json::json;
use tinyagents_harness::context::{RunConfig, RunContext};
use tinyagents_harness::no_progress::{
    DEFAULT_REPEAT_CALL_THRESHOLD, DEFAULT_REPEAT_OUTPUT_THRESHOLD,
};
use tinyinference::model::ModelRequest;

fn ctx() -> RunContext<()> {
    RunContext::new(RunConfig::new("mw-test"), ())
}

// ── payload_summarizer disclosure (#5722) ──────────────────────
//
// The behaviour these pin: when summarization does not happen, the model
// must be able to see that from the payload. Previously every one of
// these cases produced byte-identical content to a successful
// pass-through, so the model could not tell a raw dump from a normal
// result and re-called the same tool.

struct StubSummarizer(std::sync::Mutex<Option<anyhow::Result<SummarizeOutcome>>>);

impl StubSummarizer {
    fn ok(outcome: SummarizeOutcome) -> Arc<Self> {
        Arc::new(Self(std::sync::Mutex::new(Some(Ok(outcome)))))
    }
}

#[async_trait]
impl PayloadSummarizer for StubSummarizer {
    async fn maybe_summarize_in_parent(
        &self,
        _parent_ctx: &RunContext<()>,
        _tool_name: &str,
        _parent_task_hint: Option<&str>,
        _raw: &str,
    ) -> anyhow::Result<SummarizeOutcome> {
        self.0
            .lock()
            .expect("stub outcome lock")
            .take()
            .expect("stub summarizer called more than once")
    }
}

fn summarizer_mw(ps: Arc<dyn PayloadSummarizer>) -> ToolOutputMiddleware {
    ToolOutputMiddleware {
        // Large enough that the byte-budget backstop never fires, so these
        // tests observe the summarizer stage alone.
        budget_bytes: 10_000_000,
        payload_summarizer: Some(ps),
        artifact_store: None,
        tokenjuice_compaction_enabled: false,
        tokenjuice_compression:
            crate::openhuman::inference::tokenjuice::AgentTokenjuiceCompression::Off,
        tool_policies: HashMap::new(),
    }
}

/// A minimal openhuman [`Tool`] for the tool-set–backed middlewares. Its
/// `max_result_size_chars` and `external_effect` are configurable so the
/// budget/approval resolution paths can be exercised.
struct FakeTool {
    name: &'static str,
    cap: Option<usize>,
    external: bool,
}

#[async_trait]
impl Tool for FakeTool {
    fn name(&self) -> &str {
        self.name
    }
    fn description(&self) -> &str {
        "fake"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({ "type": "object" })
    }
    async fn execute(
        &self,
        _args: serde_json::Value,
    ) -> anyhow::Result<crate::openhuman::tools::ToolResult> {
        Ok(crate::openhuman::tools::ToolResult::success("ok"))
    }
    fn max_result_size_chars(&self) -> Option<usize> {
        self.cap
    }
    fn external_effect_with_args(&self, _args: &serde_json::Value) -> bool {
        self.external
    }
}

fn tool_result(name: &str, content: &str) -> TaToolResult {
    TaToolResult {
        call_id: "c1".into(),
        name: name.into(),
        content: content.into(),
        raw: None,
        error: None,
        elapsed_ms: 0,
    }
}

// ── ToolOutcomeCaptureMiddleware policy-block enrichment (issue #4094) ───

fn outcome_capture_mw() -> ToolOutcomeCaptureMiddleware {
    ToolOutcomeCaptureMiddleware::new(
        std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
        std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
    )
}

// ── ToolOutputMiddleware: COMPACTION_EXEMPT_TOOLS (workflow proposals) ───

/// A `workflow_proposal` payload with enough uniform-object rows to clear
/// tinyjuice's `MIN_ROWS` (3) and OpenHuman's default 2 KiB compaction
/// floor — i.e. exactly the shape that used to get its `"type"` marker
/// stripped by the `[json table: …]` rewrite before the middleware
/// exemption existed.
fn large_workflow_proposal_json() -> String {
    let nodes: Vec<serde_json::Value> = (0..20)
        .map(|i| {
            json!({
                "id": format!("node-{i}"),
                "kind": if i == 0 { "trigger" } else { "tool_call" },
                "name": format!("Step {i}"),
                "config": {
                    "slug": format!("oh:placeholder_action_{i}"),
                    "args": { "input": format!("value-{i}"), "note": "generic placeholder payload for size padding" }
                }
            })
        })
        .collect();
    serde_json::to_string(&json!({
        "type": "workflow_proposal",
        "flow_id": "flow-large-graph",
        "graph": { "nodes": nodes, "edges": [] },
    }))
    .unwrap()
}

fn compaction_enabled_mw() -> ToolOutputMiddleware {
    ToolOutputMiddleware {
        budget_bytes: 1_000_000,
        payload_summarizer: None,
        artifact_store: None,
        tokenjuice_compaction_enabled: true,
        tokenjuice_compression: AgentTokenjuiceCompression::Full,
        tool_policies: HashMap::new(),
    }
}

// ── ToolOutputMiddleware: truncation exemption (#4888 follow-up, gap 1) ──

/// A `workflow_proposal` payload with `node_count` nodes, each padded with
/// a 500-byte `note`, so the caller can force the serialized size past the
/// ~16 KiB shared byte-budget backstop (`DEFAULT_TOOL_RESULT_BUDGET_BYTES`)
/// — the size class a real ≥10-node graph proposal routinely reaches, and
/// exactly what used to get UTF-8-boundary-truncated into unparseable JSON
/// before the truncation exemption existed.
fn oversized_workflow_proposal_json(node_count: usize) -> String {
    let nodes: Vec<serde_json::Value> = (0..node_count)
        .map(|i| {
            json!({
                "id": format!("node-{i}"),
                "kind": if i == 0 { "trigger" } else { "tool_call" },
                "name": format!("Step {i}"),
                "config": {
                    "slug": format!("oh:placeholder_action_{i}"),
                    "args": { "input": format!("value-{i}"), "note": "a".repeat(500) }
                }
            })
        })
        .collect();
    serde_json::to_string(&json!({
        "type": "workflow_proposal",
        "flow_id": "flow-oversized-graph",
        "graph": { "nodes": nodes, "edges": [] },
    }))
    .unwrap()
}

/// Middleware config isolating the byte-cap stages (3+4): tokenjuice off,
/// no tool-declared char cap, the real `DEFAULT_TOOL_RESULT_BUDGET_BYTES`
/// (~16 KiB) as the shared backstop, and no artifact store (so an
/// over-budget non-exempt tool falls straight to inline truncation instead
/// of being persisted — deterministic to assert on).
fn truncation_probe_mw() -> ToolOutputMiddleware {
    ToolOutputMiddleware {
        budget_bytes: DEFAULT_TOOL_RESULT_BUDGET_BYTES,
        payload_summarizer: None,
        artifact_store: None,
        tokenjuice_compaction_enabled: false,
        tokenjuice_compression: AgentTokenjuiceCompression::Off,
        tool_policies: HashMap::new(),
    }
}

// ── ToolOutputMiddleware: sampling tools (#4888 follow-up, gap 2) ────────

/// A large uniform-array JSON payload shaped like a real sampled tool
/// response (no `workflow_proposal` envelope) — what `get_tool_output_sample`
/// / `get_tool_contract` actually return so the model can derive an exact
/// `primary_array_path`/`output_fields` from the real shape. `row_count`
/// rows of ≥3 clear tinyjuice's tabulation threshold.
fn large_sample_response_json(row_count: usize) -> String {
    let rows: Vec<serde_json::Value> = (0..row_count)
        .map(|i| {
            json!({
                "id": i,
                "title": format!("Issue {i}"),
                "state": "open",
                "body": "padding padding padding padding padding padding",
            })
        })
        .collect();
    serde_json::to_string(&json!({ "items": rows })).unwrap()
}

// ── RepeatedToolFailureMiddleware ───────────────────────────────────────

fn failing_result(name: &str, err: &str) -> TaToolResult {
    let mut r = tool_result(name, err);
    r.error = Some(err.to_string());
    r
}

/// Count how many of the steering commands drained from `handle` are
/// `Pause` (the halt signal). The tracker-driven breaker now also emits a
/// `Redirect` **nudge** below the retry cap, so a raw `pending()` count no
/// longer isolates the halt — the tests classify by command kind instead.
fn drain_pause_count(handle: &SteeringHandle) -> usize {
    handle
        .drain()
        .into_iter()
        .filter(|c| matches!(c, SteeringCommand::Pause))
        .count()
}

/// Collect the nudge system-message texts drained from `handle`. The nudge
/// rides the `InjectMessage` lane (not `Redirect`) so it is permitted on the
/// user's interactive turn — see the test below.
fn drain_nudge_messages(handle: &SteeringHandle) -> Vec<String> {
    handle
        .drain()
        .into_iter()
        .filter_map(|c| match c {
            SteeringCommand::InjectMessage(message) => Some(message.text()),
            _ => None,
        })
        .collect()
}

// ── RepeatedToolFailureMiddleware body-level ok:false (flows breaker) ────

/// A `ToolResult::success` (no `error`) whose JSON body carries a top-level
/// `"ok": false` — the shape `validate_workflow` / `dry_run_workflow` return
/// for an invalid graph / aborted sandbox run.
fn body_failure_result(name: &str, extra: serde_json::Value) -> TaToolResult {
    let mut body = json!({ "ok": false });
    if let serde_json::Value::Object(map) = extra {
        body.as_object_mut().unwrap().extend(map);
    }
    tool_result(name, &serde_json::to_string_pretty(&body).unwrap())
}

// ── RepeatProgressMiddleware / crate SuccessfulRepeatTracker ───────────

fn repeated_success_response(tool: &str, args: serde_json::Value) -> ModelResponse {
    ModelResponse {
        message: tinyinference::message::AssistantMessage {
            id: None,
            content: vec![ContentBlock::Text("working".to_string())],
            tool_calls: vec![TaToolCall::new("repeat-1", tool, args)],
            usage: None,
        },
        usage: None,
        finish_reason: Some("tool_calls".to_string()),
        raw: None,
        resolved_model: None,
        continue_turn: None,
        served_from_cache: false,
    }
}

async fn run_successful_repeat_cycle(
    mw: &RepeatProgressMiddleware,
    tool: &str,
    args: serde_json::Value,
    error: Option<&str>,
) {
    let mut response = repeated_success_response(tool, args);
    mw.after_model(&mut ctx(), &(), &mut response)
        .await
        .unwrap();
    let mut result = tool_result(tool, "ok");
    result.error = error.map(str::to_string);
    mw.after_tool(&mut ctx(), &(), &mut result).await.unwrap();
}

// ── MemoryProtocolMiddleware (issue #4116) ──────────────────────────────

use crate::openhuman::agent::harness::memory_protocol::MEMORY_PROTOCOL_MARKER;

/// Drive one full tool cycle through the middleware: `before_tool` (captures
/// the arguments the result won't carry) then `after_tool`, correlated by a
/// shared call id. Returns the (possibly annotated) result.
async fn run_cycle(
    mw: &MemoryProtocolMiddleware,
    name: &str,
    args: serde_json::Value,
    content: &str,
    error: Option<&str>,
) -> TaToolResult {
    let mut call = TaToolCall {
        id: "c1".into(),
        name: name.into(),
        arguments: args,
        invalid: None,
    };
    mw.before_tool(&mut ctx(), &(), &mut call).await.unwrap();
    let mut result = tool_result(name, content); // call_id "c1" matches
    result.error = error.map(|e| e.to_string());
    mw.after_tool(&mut ctx(), &(), &mut result).await.unwrap();
    result
}

// ── EmbedderToolHooksMiddleware ──────────────────────────────────────────

/// Records lifecycle notifications for a test hook, optionally vetoing every
/// pre-tool call so the veto path can be exercised.
struct RecordingToolHook {
    name: &'static str,
    pre: std::sync::Arc<std::sync::Mutex<Vec<(String, serde_json::Value)>>>,
    post: std::sync::Arc<
        std::sync::Mutex<Vec<(String, serde_json::Value, Option<bool>, Option<u64>)>>,
    >,
    veto: bool,
}

#[async_trait]
impl crate::openhuman::agent::hooks::ToolHook for RecordingToolHook {
    fn name(&self) -> &str {
        self.name
    }
    async fn before_tool(
        &self,
        context: &crate::openhuman::agent::hooks::ToolHookContext,
    ) -> anyhow::Result<()> {
        self.pre
            .lock()
            .unwrap()
            .push((context.tool_name.clone(), context.arguments.clone()));
        if self.veto {
            anyhow::bail!("vetoed by test hook");
        }
        Ok(())
    }
    async fn after_tool(
        &self,
        context: &crate::openhuman::agent::hooks::ToolHookContext,
    ) -> anyhow::Result<()> {
        self.post.lock().unwrap().push((
            context.tool_name.clone(),
            context.arguments.clone(),
            context.success,
            context.duration_ms,
        ));
        Ok(())
    }
}

fn embedder_hook_mw(
    pre: std::sync::Arc<std::sync::Mutex<Vec<(String, serde_json::Value)>>>,
    post: std::sync::Arc<
        std::sync::Mutex<Vec<(String, serde_json::Value, Option<bool>, Option<u64>)>>,
    >,
    veto: bool,
) -> EmbedderToolHooksMiddleware {
    EmbedderToolHooksMiddleware::new(vec![std::sync::Arc::new(RecordingToolHook {
        name: "recording",
        pre,
        post,
        veto,
    })])
}

#[path = "middleware_tests_part_01_tests.rs"]
mod part_01_tests;
#[path = "middleware_tests_part_02_tests.rs"]
mod part_02_tests;
#[path = "middleware_tests_part_03_tests.rs"]
mod part_03_tests;
#[path = "middleware_tests_part_04_tests.rs"]
mod part_04_tests;
