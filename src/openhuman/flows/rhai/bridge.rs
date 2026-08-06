//! Capability bridge: projects openhuman's tools, provider model, and
//! sub-agents into a `tinyagents` [`CapabilityRegistry`] a `.ragsh` session
//! binds against.
//!
//! Three capability kinds are wired, each keeping openhuman's own gates:
//!
//! - **Tools** — one [`RhaiToolAdapter`] per visible, non-excluded, agent-scoped
//!   tool. Approval is **not** on the tinyagents repl path (it lives in the
//!   harness `wrap_tool` middleware the REPL bypasses), so the adapter itself
//!   invokes the [`ApprovalGate`] for any tool whose `external_effect_with_args`
//!   is true, failing closed on denial and recording the terminal outcome.
//! - **Model** — the turn's provider, registered under its model name so
//!   `model_query(#{model: "<name>"})` hits the real backend with usage intact.
//! - **Agents** — a [`SubagentCapability`] per entry in the parent's
//!   `allowed_subagent_ids`, so `agent_query("<id>", ...)` spawns a real
//!   openhuman sub-agent through `run_subagent`.
//!
//! Recursion/duplication hazards are **excluded** from the tool surface: `rhai`
//! itself (no REPL-in-REPL), `spawn_*` (use `agent_query`), and
//! `run_workflow`/`await_workflow`. `ToolScope::CliRpcOnly` tools are excluded
//! too.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use tinyagents::graph::subagent_node::{HarnessAgent, SubAgentInput, SubAgentOutput};
use tinyagents::harness::events::EventSink;
use tinyagents::harness::tool::{
    Tool as TaTool, ToolCall as TaToolCall, ToolExecutionContext, ToolPolicy as TaToolPolicy,
    ToolResult as TaToolResult, ToolSchema as TaToolSchema,
};
use tinyagents::registry::CapabilityRegistry;
use tinyagents::TinyAgentsError;

use crate::openhuman::agent::harness::definition::AgentDefinitionRegistry;
use crate::openhuman::agent::harness::fork_context::{with_parent_context, ParentExecutionContext};
use crate::openhuman::agent::harness::subagent_runner::{run_subagent, SubagentRunOptions};
use crate::openhuman::agent::tinyagents::tools::{
    execute_openhuman_tool, tool_policy_from_openhuman_tool,
};
use crate::openhuman::security::approval::{
    redact_args, summarize_action, ApprovalGate, ExecutionOutcome, GateOutcome,
};
use crate::openhuman::tools::traits::ToolScope;
use crate::openhuman::tools::Tool as OhTool;

/// Tools never exposed to a `.ragsh` script, to prevent recursion (a script
/// re-entering the REPL) and capability duplication (spawn/workflow primitives
/// the script models with `agent_query` instead).
fn is_excluded_tool(name: &str) -> bool {
    name == "rhai_workflows"
        || name == "rhai"
        || name == "rlm"
        || name == "run_workflow"
        || name == "await_workflow"
        || name.starts_with("spawn_")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::tools::traits::ToolResult as OhToolResult;

    #[test]
    fn recursion_and_duplication_hazards_are_excluded() {
        for name in [
            "rhai",
            "rhai_workflows",
            "rlm",
            "run_workflow",
            "await_workflow",
            "spawn_subagent",
            "spawn_parallel_agents",
            "spawn_async_subagent",
        ] {
            assert!(is_excluded_tool(name), "{name} should be excluded");
        }
        for name in ["read_file", "grep", "edit_file", "web_search"] {
            assert!(!is_excluded_tool(name), "{name} should be callable");
        }
    }

    // ── Per-cell outcome tracking (E-m5) ──────────────────────────────────

    #[test]
    fn outcome_tracking_round_trips_and_resets() {
        // Untracked (no `begin_call_outcome_tracking`) is a safe no-op.
        record_call_outcome("orphan", false);
        assert!(take_call_outcomes().is_empty());

        begin_call_outcome_tracking();
        record_call_outcome("a", true);
        record_call_outcome("b", false);
        let outcomes = take_call_outcomes();
        assert_eq!(outcomes.get("a"), Some(&true));
        assert_eq!(outcomes.get("b"), Some(&false));

        // `take` ends tracking — a later record without a new `begin` is a
        // no-op again, so it can never bleed into the next cell on the same
        // (possibly reused) thread-pool thread.
        record_call_outcome("c", true);
        assert!(take_call_outcomes().is_empty());
    }

    /// A minimal openhuman [`OhTool`] whose success/failure is driven by its
    /// `fail` argument, for exercising [`RhaiToolAdapter::dispatch`] end to
    /// end.
    struct FlakyTool;

    #[async_trait]
    impl OhTool for FlakyTool {
        fn name(&self) -> &str {
            "flaky"
        }
        fn description(&self) -> &str {
            "fails when called with fail: true"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({ "type": "object" })
        }
        async fn execute(&self, args: serde_json::Value) -> anyhow::Result<OhToolResult> {
            if args.get("fail").and_then(serde_json::Value::as_bool) == Some(true) {
                Ok(OhToolResult::error("boom"))
            } else {
                Ok(OhToolResult::json(serde_json::json!({ "ok": true })))
            }
        }
    }

    fn flaky_adapter() -> RhaiToolAdapter {
        let tools: Arc<Vec<Box<dyn OhTool>>> = Arc::new(vec![Box::new(FlakyTool)]);
        RhaiToolAdapter::new(tools.clone(), tools[0].as_ref())
    }

    fn call(id: &str, fail: bool) -> TaToolCall {
        TaToolCall {
            id: id.to_string(),
            name: "flaky".to_string(),
            arguments: serde_json::json!({ "fail": fail }),
            invalid: None,
        }
    }

    #[tokio::test]
    async fn dispatch_records_a_successful_call_as_ok() {
        let adapter = flaky_adapter();
        begin_call_outcome_tracking();
        let result = adapter.dispatch(call("c-ok", false), None).await;
        assert!(result.error.is_none());
        assert_eq!(take_call_outcomes().get("c-ok"), Some(&true));
    }

    #[tokio::test]
    async fn dispatch_records_a_failed_call_as_not_ok() {
        let adapter = flaky_adapter();
        begin_call_outcome_tracking();
        let result = adapter.dispatch(call("c-fail", true), None).await;
        assert!(result.error.is_some());
        assert_eq!(
            take_call_outcomes().get("c-fail"),
            Some(&false),
            "a tool-reported failure must be tracked as not ok even though the vendor REPL \
             records the call regardless of `result.error`"
        );
    }

    #[tokio::test]
    async fn dispatch_records_an_unknown_tool_as_not_ok() {
        let tools: Arc<Vec<Box<dyn OhTool>>> = Arc::new(Vec::new());
        let missing_tool = FlakyTool;
        let adapter = RhaiToolAdapter::new(tools, &missing_tool);
        begin_call_outcome_tracking();
        let result = adapter.dispatch(call("c-missing", false), None).await;
        assert!(result.error.is_some());
        assert_eq!(take_call_outcomes().get("c-missing"), Some(&false));
    }
}

// ── Per-cell tool-call outcome tracking (E-m5) ──────────────────────────────
//
// The vendor REPL's `tool_call_impl` records a `ReplCallRecord` for a tool
// call *before* it checks whether the tool itself reported an error (so a
// try/catch-wrapped failure still lands in `ReplResult::calls`), and
// `tool_call_batched_impl` records + keeps going on a per-item tool failure
// without ever raising at all. `ReplCallRecord` carries no success flag
// either way, so `ops::summarize_calls` cannot tell a caught/per-batch-item
// failure from a success by looking at the vendor record alone. This tracks
// the real outcome on our side, keyed by the same `call_id` the vendor record
// carries, as each call is dispatched through [`RhaiToolAdapter::dispatch`].

thread_local! {
    /// Per-cell outcome map (`call_id` -> `ok`). `None` means "not currently
    /// tracking" (a call dispatched outside an `eval_cell` — defensive only,
    /// should not happen) so `record_call_outcome` is a safe no-op; `ops.rs`
    /// defaults an untracked `call_id` to `ok: true`, matching every other
    /// capability kind (model/agent/graph/emit), which the vendor session
    /// only ever records on success.
    static CALL_OUTCOMES: RefCell<Option<HashMap<String, bool>>> = const { RefCell::new(None) };
}

/// Begins per-cell tool-call outcome tracking on the current thread. Call
/// once, immediately before `ReplSession::eval_cell`, matched by
/// [`take_call_outcomes`] after it returns.
///
/// Confined to a single OS thread by construction: `eval_cell` and every tool
/// dispatch it drives run synchronously on the `spawn_blocking` thread that
/// calls this — the vendor REPL's capability bridge blocks that same thread
/// to completion (`futures::executor::block_on`, see
/// `session/builtins/mod.rs`'s module doc) rather than yielding to another
/// worker thread.
pub(super) fn begin_call_outcome_tracking() {
    CALL_OUTCOMES.with(|cell| *cell.borrow_mut() = Some(HashMap::new()));
}

/// Ends tracking and returns everything recorded since the matching
/// [`begin_call_outcome_tracking`] (empty if tracking was never begun).
pub(super) fn take_call_outcomes() -> HashMap<String, bool> {
    CALL_OUTCOMES
        .with(|cell| cell.borrow_mut().take())
        .unwrap_or_default()
}

fn record_call_outcome(call_id: &str, ok: bool) {
    CALL_OUTCOMES.with(|cell| {
        if let Some(map) = cell.borrow_mut().as_mut() {
            map.insert(call_id.to_string(), ok);
        }
    });
}

/// Builds the `CapabilityRegistry<()>` a session binds against from the parent
/// turn's execution context.
///
/// Reads the parent's visible tool set, provider/model, and sub-agent
/// allowlist. The returned registry carries no `rhai`, `spawn_*`, or workflow
/// tools, and no `CliRpcOnly`-scoped tools.
pub(super) fn build_capability_registry(
    parent: &ParentExecutionContext,
) -> anyhow::Result<CapabilityRegistry<()>> {
    let mut registry = CapabilityRegistry::<()>::new();

    // ── Model: the turn's provider, under its registered name. ──
    let model = parent
        .turn_model_source
        .build_summarizer(&parent.model_name, parent.temperature)?;
    registry.replace_model(parent.model_name.clone(), model);

    // ── Tools: visible, non-excluded, agent-scoped only. ──
    let mut tool_count = 0usize;
    for tool in parent.all_tools.iter() {
        let name = tool.name();
        if !parent.visible_tool_names.is_empty() && !parent.visible_tool_names.contains(name) {
            continue;
        }
        if is_excluded_tool(name) {
            continue;
        }
        if matches!(tool.scope(), ToolScope::CliRpcOnly) {
            continue;
        }
        registry.replace_tool(Arc::new(RhaiToolAdapter::new(
            parent.all_tools.clone(),
            tool.as_ref(),
        )));
        tool_count += 1;
    }

    // ── Agents: one capability per allowed sub-agent id. ──
    let mut agent_count = 0usize;
    for agent_id in &parent.allowed_subagent_ids {
        registry.replace_agent(Arc::new(SubagentCapability {
            agent_id: agent_id.clone(),
            parent: parent.clone(),
        }));
        agent_count += 1;
    }

    tracing::debug!(
        tools = tool_count,
        agents = agent_count,
        model = %parent.model_name,
        "[rhai_workflows] built capability registry"
    );
    Ok(registry)
}

/// A `tinyagents` tool backed by an openhuman [`Tool`](OhTool), located by name
/// in the parent's shared tool set on each call (the set is `Arc`-shared, not
/// cloned). Adds the approval gate the harness `wrap_tool` middleware would
/// otherwise apply — absent on the repl bridge path.
pub(super) struct RhaiToolAdapter {
    tools: Arc<Vec<Box<dyn OhTool>>>,
    name: String,
    description: String,
    schema: TaToolSchema,
    policy: TaToolPolicy,
}

impl RhaiToolAdapter {
    fn new(tools: Arc<Vec<Box<dyn OhTool>>>, tool: &dyn OhTool) -> Self {
        let schema = TaToolSchema {
            name: tool.name().to_string(),
            description: tool.description().to_string(),
            parameters: tool.parameters_schema(),
            format: Default::default(),
        };
        Self {
            name: tool.name().to_string(),
            description: tool.description().to_string(),
            schema,
            policy: tool_policy_from_openhuman_tool(tool),
            tools,
        }
    }

    async fn dispatch(
        &self,
        call: TaToolCall,
        context: Option<&ToolExecutionContext>,
    ) -> TaToolResult {
        let found = self.tools.iter().find(|t| t.name() == self.name);
        let result = match found {
            Some(tool) => gated_execute(tool.as_ref(), call, context).await,
            None => {
                tracing::warn!(tool = %self.name, "[rhai_workflows] bridged tool not found at call time");
                TaToolResult {
                    call_id: call.id,
                    name: call.name,
                    content: format!("Error: unknown tool '{}'", self.name),
                    raw: None,
                    error: Some("unknown tool".to_string()),
                    elapsed_ms: 0,
                }
            }
        };
        // Record the real outcome (E-m5) — the vendor REPL records a
        // `ReplCallRecord` for this call_id regardless of `result.error`, so
        // `ops::summarize_calls` needs this side channel to report a
        // caught/per-batch-item tool failure as `ok: false` instead of the
        // vendor type's implicit (wrong) "recorded == succeeded".
        record_call_outcome(&result.call_id, result.error.is_none());
        result
    }
}

#[async_trait]
impl TaTool<()> for RhaiToolAdapter {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn schema(&self) -> TaToolSchema {
        self.schema.clone()
    }

    fn policy(&self) -> TaToolPolicy {
        self.policy.clone()
    }

    async fn call(&self, _state: &(), call: TaToolCall) -> tinyagents::Result<TaToolResult> {
        Ok(self.dispatch(call, None).await)
    }

    async fn call_with_context(
        &self,
        _state: &(),
        call: TaToolCall,
        context: ToolExecutionContext,
    ) -> tinyagents::Result<TaToolResult> {
        Ok(self.dispatch(call, Some(&context)).await)
    }
}

/// Runs an openhuman tool for a `.ragsh` `tool_call`, routing any external-effect
/// tool through the [`ApprovalGate`] first (fail-closed on denial) since the
/// repl bridge sits outside the harness approval middleware.
///
/// **E-m4: a Supervised-tier park from inside a cell is practically
/// unanswerable.** This calls [`ApprovalGate::intercept_audited`] — the
/// UNBOUNDED variant, which awaits up to the gate's own TTL (10 minutes by
/// default, `DEFAULT_APPROVAL_TTL` in `approval/gate.rs`) — from inside a
/// Rhai cell whose own wall-clock deadline is `rhai_workflows::policy`'s
/// `DEFAULT_RHAI_TIMEOUT_SECS` (300s, i.e. 5 minutes) unless the caller
/// passed a longer `timeout_secs`. With the default cell timeout, the cell
/// times out and is torn down at 5 minutes — well before a human could
/// plausibly see, read, and act on the approval card — so in practice a
/// Supervised-tier `tool_call`/`code`/native-tool capability invoked from a
/// cell either gets approved within the cell's own timeout window (a very
/// fast human response) or the cell dies waiting, and the approval decision
/// (if it eventually comes) lands on a session and cell that no longer
/// exist. Compounding this: per E-M1/E-M2's session-timeout-ordering bugs,
/// a cell that times out this way can also lose the whole Rhai session's
/// bindings, not just this one call.
///
/// [`ApprovalGate::intercept_audited_bounded`] exists precisely to cap the
/// park window below a caller's own deadline — this bridge does not use it,
/// which is the gap this comment documents rather than fixes (bounding the
/// park to the cell's remaining timeout, or refusing to park at all from
/// inside a cell, is tracked as follow-up, not applied in this doc-only
/// pass).
async fn gated_execute(
    tool: &dyn OhTool,
    call: TaToolCall,
    context: Option<&ToolExecutionContext>,
) -> TaToolResult {
    if tool.external_effect_with_args(&call.arguments) {
        if let Some(gate) = ApprovalGate::try_global() {
            let summary = summarize_action(&call.name, &call.arguments);
            let redacted = redact_args(&call.arguments);
            tracing::debug!(tool = %call.name, "[rhai_workflows] external-effect tool — routing through approval gate");
            let (outcome, request_id) =
                gate.intercept_audited(&call.name, &summary, redacted).await;
            match outcome {
                GateOutcome::Deny { reason } => {
                    tracing::info!(tool = %call.name, %reason, "[rhai_workflows] tool denied by approval gate");
                    return TaToolResult {
                        content: format!("Denied by approval gate: {reason}"),
                        error: Some(format!("approval denied: {reason}")),
                        raw: None,
                        elapsed_ms: 0,
                        call_id: call.id,
                        name: call.name,
                    };
                }
                GateOutcome::Allow => {
                    let result = execute_openhuman_tool(tool, call, context).await;
                    if let Some(id) = request_id {
                        let terminal = if result.error.is_none() {
                            ExecutionOutcome::Success
                        } else {
                            ExecutionOutcome::Failure
                        };
                        gate.record_execution(&id, terminal, result.error.as_deref());
                    }
                    return result;
                }
            }
        }
        // No global gate installed (e.g. gate disabled): fall through and
        // execute — the harness-level env kill-switch owns that decision.
    }
    execute_openhuman_tool(tool, call, context).await
}

/// A `.ragsh` `agent_query("<id>", ...)` capability that spawns a real openhuman
/// sub-agent via `run_subagent`.
///
/// Captures the parent [`ParentExecutionContext`] at bridge-build time and
/// **re-installs it** with [`with_parent_context`] before calling
/// `run_subagent`: the session's `eval_cell` runs on `spawn_blocking` +
/// `futures::executor::block_on`, which does not carry the `PARENT_CONTEXT`
/// task-local `run_subagent` resolves — without this the spawn would fail with
/// `NoParentContext`.
struct SubagentCapability {
    agent_id: String,
    parent: ParentExecutionContext,
}

#[async_trait]
impl HarnessAgent for SubagentCapability {
    fn name(&self) -> &str {
        &self.agent_id
    }

    async fn run(
        &self,
        input: SubAgentInput,
        _events: EventSink,
    ) -> tinyagents::Result<SubAgentOutput> {
        let registry = AgentDefinitionRegistry::global().ok_or_else(|| {
            TinyAgentsError::Capability("agent registry not initialised".to_string())
        })?;
        let definition = registry.get(&self.agent_id).ok_or_else(|| {
            TinyAgentsError::Capability(format!("agent `{}` is not registered", self.agent_id))
        })?;
        // Defensive re-check of the parent allowlist (the bridge only registers
        // allowed agents, but never trust that a script cannot reach further).
        if !self.parent.allowed_subagent_ids.contains(&definition.id) {
            return Err(TinyAgentsError::Capability(format!(
                "agent `{}` is not in the parent's subagent allowlist",
                definition.id
            )));
        }

        let options = SubagentRunOptions {
            workspace_descriptor: self.parent.workspace_descriptor.clone(),
            ..Default::default()
        };

        tracing::debug!(agent = %self.agent_id, "[rhai_workflows] agent_query — spawning sub-agent");
        let outcome = with_parent_context(
            self.parent.clone(),
            run_subagent(definition, &input.prompt, options),
        )
        .await
        .map_err(|e| TinyAgentsError::Tool(format!("sub-agent `{}` failed: {e}", self.agent_id)))?;

        Ok(SubAgentOutput {
            text: outcome.output,
            model_calls: outcome.iterations,
            ..Default::default()
        })
    }
}
