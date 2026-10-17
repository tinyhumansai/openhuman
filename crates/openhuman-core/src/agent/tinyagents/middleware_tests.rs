use super::approval::approval_tool_name;
use super::repeated_failure::{is_body_level_failure, user_actionable_escalation};
use super::tool_output::{
    is_compaction_exempt, is_truncation_exempt, COMPACTION_EXEMPT_TOOLS, SAMPLING_TOOLS,
};
use super::turn_context::DEFAULT_TOOL_RESULT_BUDGET_BYTES;
use crate::agent::tinyagents::middleware::embedder_hooks::EmbedderToolHooksMiddleware;
use crate::agent::tinyagents::middleware::repeated_failure::RepeatedToolFailureMiddleware;
use crate::agent::tinyagents::middleware::tool_outcome_capture::ToolOutcomeCaptureMiddleware;
use crate::agent::tinyagents::middleware::tool_output::ToolOutputMiddleware;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;

use tinyagents_harness::middleware::{AgentRun, Middleware, ToolInvocationIdentity};
use tinyagents_harness::steering::{SteeringCommand, SteeringHandle};
use tinyinference_llm::message::Message as TaMessage;
use tinyinference_llm::tool::ToolCall as TaToolCall;
use tinytools::{ToolPolicy as TaToolPolicy, ToolResult as TaToolResult};

use crate::agent::context::CLEARED_PLACEHOLDER;
use crate::agent::tinyagents::payload_summarizer::DynPayloadSummarizer;
use tinyagents_harness::context::{RunConfig, RunContext};
use tinyinference_llm::model::ModelRequest;
use tinyjuice_bus::types::AgentTokenjuiceCompression;
use tinytools::Tool;

fn ctx() -> RunContext<crate::agent::tinyagents::host::OpenHumanRunContext> {
    RunContext::new(
        RunConfig::new("mw-test"),
        crate::agent::tinyagents::host::OpenHumanRunContext::new(),
    )
}

// ── TinyJuice summary disclosure (#5722) ───────────────────────────────
//
// The behaviour these pin: when summarization does not happen, the model
// must be able to see that from the payload. Previously every one of
// these cases produced byte-identical content to a successful
// pass-through, so the model could not tell a raw dump from a normal
// result and re-called the same tool. The module itself is stood in for by
// `tokenjuice::module_stub`, which calls back through `MlHost.Generate`
// the way TinyJuice does.

/// A summary model that answers every call with one fixed reply.
struct StubSummarizer {
    reply: std::sync::Mutex<Option<anyhow::Result<String>>>,
    prepared: std::sync::atomic::AtomicBool,
}

impl StubSummarizer {
    fn replying(reply: anyhow::Result<String>) -> Arc<Self> {
        Arc::new(Self {
            reply: std::sync::Mutex::new(Some(reply)),
            prepared: Default::default(),
        })
    }

    /// Whether the middleware bound a summary call for this result at all.
    fn was_prepared(&self) -> bool {
        self.prepared.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl
    tinyagents_harness::summarization::payload_summarizer::PayloadSummarizer<
        crate::agent::tinyagents::host::OpenHumanRunContext,
    > for StubSummarizer
{
    type Prepared = crate::inference::tokenjuice::generate::PreparedGenerate;
    type Error = anyhow::Error;

    fn prepare(
        &self,
        _parent_ctx: &RunContext<crate::agent::tinyagents::host::OpenHumanRunContext>,
    ) -> anyhow::Result<crate::inference::tokenjuice::generate::PreparedGenerate> {
        self.prepared
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let reply = self
            .reply
            .lock()
            .expect("stub reply lock")
            .take()
            .expect("stub summarizer prepared more than once");
        Ok(Box::new(move |_request| Box::pin(async move { reply })))
    }
}

/// A config whose summary threshold every test payload clears.
fn summarizing_config() -> Arc<crate::config::Config> {
    let mut config = crate::config::Config::default();
    config.context.summarizer_payload_threshold_tokens = 1;
    Arc::new(config)
}

fn summarizer_mw(ps: Arc<DynPayloadSummarizer>) -> ToolOutputMiddleware {
    ToolOutputMiddleware {
        // Large enough that the byte-budget backstop never fires, so these
        // tests observe the summary stage alone.
        budget_bytes: 10_000_000,
        payload_summarizer: Some(ps),
        artifact_store: None,
        tokenjuice_compaction_enabled: false,
        tokenjuice_compression: tinyjuice_bus::types::AgentTokenjuiceCompression::Full,
        runtime_config: Some(summarizing_config()),
        tool_policies: HashMap::new(),
        artifact_reads: Default::default(),
        focus_by_call: Default::default(),
        // `web_fetch` declares `summary_focus` in production.
        summary_focus_tools: ["web_fetch".to_string()].into(),
        raw_fetches: Default::default(),
        file_reads: Default::default(),
    }
}

/// Run `fut` against a stubbed TinyJuice module, returning what it was sent.
async fn with_module<F: std::future::Future>(
    fut: F,
) -> (F::Output, Vec<tinyjuice_bus::wire::CompactRequest>) {
    use crate::inference::tokenjuice::module_stub;
    let recorded = Arc::new(std::sync::Mutex::new(Vec::new()));
    let output = module_stub::with_stub(module_stub::summarizing(recorded.clone()), fut).await;
    let requests = recorded.lock().unwrap().clone();
    (output, requests)
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
    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<tinytools::ToolResult> {
        Ok(tinytools::ToolResult::success("ok"))
    }
    fn max_result_size_chars(&self) -> Option<usize> {
        self.cap
    }
    fn external_effect_with_args(&self, _args: &serde_json::Value) -> bool {
        self.external
    }
}

fn tool_result(_name: &str, content: &str) -> TaToolResult {
    TaToolResult::success(content)
}

fn invocation(call_id: impl AsRef<str>, tool_name: impl Into<String>) -> ToolInvocationIdentity {
    ToolInvocationIdentity::new(
        tinyagents_harness::ids::CallId::new(call_id.as_ref()),
        tool_name,
    )
}

fn result_text(result: &TaToolResult) -> String {
    result.output()
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
        runtime_config: None,
        tool_policies: HashMap::new(),
        artifact_reads: Default::default(),
        focus_by_call: Default::default(),
        summary_focus_tools: Default::default(),
        raw_fetches: Default::default(),
        file_reads: Default::default(),
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
        runtime_config: None,
        tool_policies: HashMap::new(),
        artifact_reads: Default::default(),
        focus_by_call: Default::default(),
        summary_focus_tools: Default::default(),
        raw_fetches: Default::default(),
        file_reads: Default::default(),
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
    let _ = name;
    TaToolResult::error(err)
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

/// Collect the nudge texts queued for the next model request. Nudges are
/// request-scoped (#6725): they never ride a steering command.
fn drain_nudge_messages(mw: &RepeatedToolFailureMiddleware) -> Vec<String> {
    mw.take_pending_nudges()
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

// ── EmbedderToolHooksMiddleware ──────────────────────────────────────────

/// One recorded post-tool notification: tool name, args, success flag, duration.
type PostToolRecord = (String, serde_json::Value, Option<bool>, Option<u64>);

/// Records lifecycle notifications for a test hook, optionally vetoing every
/// pre-tool call so the veto path can be exercised.
struct RecordingToolHook {
    name: &'static str,
    pre: std::sync::Arc<std::sync::Mutex<Vec<(String, serde_json::Value)>>>,
    post: std::sync::Arc<std::sync::Mutex<Vec<PostToolRecord>>>,
    veto: bool,
}

#[async_trait]
impl crate::agent::hooks::ToolHook for RecordingToolHook {
    fn name(&self) -> &str {
        self.name
    }
    async fn before_tool(
        &self,
        context: &crate::agent::hooks::ToolHookContext,
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
        context: &crate::agent::hooks::ToolHookContext,
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
    post: std::sync::Arc<std::sync::Mutex<Vec<PostToolRecord>>>,
    veto: bool,
) -> EmbedderToolHooksMiddleware {
    EmbedderToolHooksMiddleware::new(vec![std::sync::Arc::new(RecordingToolHook {
        name: "recording",
        pre,
        post,
        veto,
    })])
}

#[path = "middleware_approval_guard_tests.rs"]
mod approval_guard_tests;
#[path = "middleware_classified_failure_fetch_tests.rs"]
mod classified_failure_fetch_tests;
#[path = "middleware_classified_failure_tests.rs"]
mod classified_failure_tests;
#[path = "middleware_command_exit_failure_tests.rs"]
mod command_exit_failure_tests;
#[path = "middleware_failure_effect_tests.rs"]
mod failure_effect_tests;
#[path = "middleware_failure_policy_class_tests.rs"]
mod failure_policy_class_tests;
#[path = "middleware_loop_guard_tests.rs"]
mod loop_guard_tests;
#[path = "middleware_memory_budget_tests.rs"]
mod memory_budget_tests;
#[path = "middleware_stop_cause_tests.rs"]
mod stop_cause_tests;

#[path = "middleware_memory_and_hooks_tests.rs"]
mod memory_and_hooks_tests;
#[path = "middleware_tool_output_artifact_tests.rs"]
mod tool_output_artifact_tests;
#[path = "middleware_tool_output_tests.rs"]
mod tool_output_tests;
#[path = "middleware_tool_policy_tests.rs"]
mod tool_policy_tests;
