//! openhuman context concerns expressed as tinyagents graph middlewares
//! (issue #4249).
//!
//! Historically these ran in the in-house engine's tool/prompt plumbing
//! (`agent_tool_exec`, `ContextManager`). The tinyagents turn path bypassed
//! them, so they were effectively dead on the live loop. Re-expressing them as
//! [`Middleware`] hooks restores the behaviour and makes the graph the single
//! place cross-cutting context concerns live:
//!
//! - [`CacheAlignMiddleware`] (`before_model`) — warn on volatile tokens in the
//!   system prompt that would bust the provider KV-cache prefix. Warn-only.
//! - [`MicrocompactMiddleware`] (`before_model`) — clear the bodies of older
//!   tool-result messages (keeping the N most recent) so a long tool-heavy
//!   thread stays cheap without dropping chat history.
//! - [`ToolOutputMiddleware`] (`after_tool`) — apply the per-tool-result byte
//!   cap and (optionally) the semantic payload summarizer to each tool result
//!   as it returns, before it enters the transcript.
//!
//! [`TurnContextMiddleware`] bundles the config and installs whichever hooks are
//! enabled onto a harness.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;

use tinyagents::error::Result as TaResult;
use tinyagents::harness::context::RunContext;
use tinyagents::harness::message::{ContentBlock, Message as TaMessage};
use tinyagents::harness::middleware::{
    Middleware, MiddlewareToolOutcome, ToolHandler, ToolMiddleware,
};
use tinyagents::harness::model::ModelRequest;
use tinyagents::harness::runtime::AgentHarness;
use tinyagents::harness::steering::{SteeringCommand, SteeringHandle};
use tinyagents::harness::tool::{ToolCall as TaToolCall, ToolResult as TaToolResult};

use super::tools::UNKNOWN_TOOL_SENTINEL;
use crate::openhuman::agent::harness::payload_summarizer::PayloadSummarizer;
use crate::openhuman::approval::{
    redact_args, summarize_action, ApprovalGate, ExecutionOutcome, GateOutcome,
};
use crate::openhuman::context::tool_result_budget::apply_tool_result_budget;
use crate::openhuman::context::CLEARED_PLACEHOLDER;
use crate::openhuman::tools::Tool;

/// Default per-tool-result byte cap for the channel / sub-agent paths, which do
/// not carry a session `ContextManager` to source the configured budget from.
/// Mirrors the `ContextConfig::tool_result_budget_bytes` default (16 KiB).
pub const DEFAULT_TOOL_RESULT_BUDGET_BYTES: usize = 16 * 1024;

/// Config bundle for the openhuman context middlewares installed on a turn.
///
/// Cheap to clone (the summarizer is an `Arc`). An all-default value installs
/// nothing — [`install`](Self::install) is a no-op.
#[derive(Clone, Default)]
pub struct TurnContextMiddleware {
    /// Per-tool-result byte cap. `0` disables the cap.
    pub tool_result_budget_bytes: usize,
    /// Optional semantic tool-output summarizer (progressive disclosure).
    pub payload_summarizer: Option<Arc<dyn PayloadSummarizer>>,
    /// Warn on volatile tokens in the system prompt (KV-cache diagnostic).
    pub cache_align: bool,
    /// Keep-recent count for microcompact tool-body clearing. `0` disables it.
    pub microcompact_keep_recent: usize,
    /// Whether the LLM summarization step (`ContextCompressionMiddleware`) may be
    /// installed on this turn. `false` when `[context].enabled` or
    /// `autocompact_enabled` is off, so a diagnostic/test opt-out doesn't spend
    /// summarizer tokens or rewrite history. The deterministic hard-trim backstop
    /// still installs regardless. Defaults to `true` (see [`defaults`](Self::defaults)).
    pub autocompact_enabled: bool,
    /// "Super context" first-turn context collection. `Some` installs the
    /// [`SuperContextMiddleware`] graph node; `None` (the default, and every
    /// non-chat path) skips it. Only the chat turn sets this — and only when its
    /// gate (`should_run_super_context`) passes.
    pub super_context: Option<SuperContextConfig>,
    /// Progressive-disclosure handoff: when set (integrations_agent with a
    /// resolved toolkit), oversized tool results are stashed in the shared
    /// [`ResultHandoffCache`] and replaced with an `extract_from_result` drill-in
    /// placeholder. `None` everywhere else.
    pub handoff: Option<HandoffConfig>,
}

/// Config for the [`HandoffMiddleware`]: the per-spawn cache (shared with the
/// `extract_from_result` tool) plus the ids used in handoff log lines.
#[derive(Clone)]
pub struct HandoffConfig {
    pub cache: Arc<crate::openhuman::agent::harness::subagent_runner::ResultHandoffCache>,
    pub agent_id: String,
    pub task_id: String,
}

/// Inputs the [`SuperContextMiddleware`] node needs to run its first-turn
/// read-only context-collection pass.
#[derive(Clone)]
pub struct SuperContextConfig {
    /// The raw user ask, used as the context scout's query.
    pub user_message: String,
}

impl TurnContextMiddleware {
    /// A sensible default for turn paths without a session `ContextManager`
    /// (channel / sub-agent): cache-align warnings on and the default tool-result
    /// byte cap, no summarizer or microcompact.
    pub fn defaults() -> Self {
        Self {
            tool_result_budget_bytes: DEFAULT_TOOL_RESULT_BUDGET_BYTES,
            payload_summarizer: None,
            cache_align: true,
            microcompact_keep_recent: 0,
            autocompact_enabled: true,
            super_context: None,
            handoff: None,
        }
    }

    /// `true` when no middleware would be installed.
    pub fn is_empty(&self) -> bool {
        self.tool_result_budget_bytes == 0
            && self.payload_summarizer.is_none()
            && !self.cache_align
            && self.microcompact_keep_recent == 0
            && self.super_context.is_none()
    }

    /// Push the enabled middlewares onto `harness`.
    ///
    /// `before_model` hooks run in registration order, so cache-align (warn) and
    /// microcompact (clear tool bodies) are installed **before** the caller's
    /// summarization / trim middlewares — microcompact frees cheap tokens first,
    /// then summarization/trim handle the rest.
    pub fn install(self, harness: &mut AgentHarness<()>, tool_sets: &[Arc<Vec<Box<dyn Tool>>>]) {
        // Super context runs first: it prepares the read-only context bundle and
        // folds it into the first model call's user message before any other
        // before_model hook inspects the request.
        if let Some(sc) = self.super_context {
            harness.push_middleware(Arc::new(SuperContextMiddleware {
                user_message: sc.user_message,
                ran: AtomicBool::new(false),
            }));
        }
        if self.cache_align {
            harness.push_middleware(Arc::new(CacheAlignMiddleware));
        }
        if self.microcompact_keep_recent > 0 {
            harness.push_middleware(Arc::new(MicrocompactMiddleware {
                keep_recent: self.microcompact_keep_recent,
            }));
        }
        // Handoff runs BEFORE the tool-output budget so an oversized payload is
        // stashed + replaced with a short placeholder first; the byte cap would
        // otherwise shrink it below the handoff threshold and defeat the drill-in.
        if let Some(handoff) = self.handoff {
            harness.push_middleware(Arc::new(HandoffMiddleware {
                cache: handoff.cache,
                agent_id: handoff.agent_id,
                task_id: handoff.task_id,
            }));
        }
        if self.tool_result_budget_bytes > 0 || self.payload_summarizer.is_some() {
            harness.push_middleware(Arc::new(ToolOutputMiddleware {
                budget_bytes: self.tool_result_budget_bytes,
                payload_summarizer: self.payload_summarizer,
                tool_sets: tool_sets.to_vec(),
            }));
        }
    }
}

/// `after_tool`: progressive-disclosure handoff (issue #4249 1b). An oversized
/// sub-agent tool result is stashed in the shared [`ResultHandoffCache`] and its
/// content replaced with a short placeholder naming a `result_id` the model can
/// drill into via `extract_from_result`. Restores the seam the legacy
/// `SubagentToolSource` ran on every tool result (via `apply_handoff`), which the
/// agent_graph rewrite dropped. Errors and `extract_from_result`'s own output
/// pass through unchanged (handled inside `apply_handoff`).
pub struct HandoffMiddleware {
    cache: Arc<crate::openhuman::agent::harness::subagent_runner::ResultHandoffCache>,
    agent_id: String,
    task_id: String,
}

#[async_trait]
impl Middleware<()> for HandoffMiddleware {
    fn name(&self) -> &str {
        "result_handoff"
    }

    async fn after_tool(
        &self,
        _ctx: &mut RunContext<()>,
        _state: &(),
        result: &mut TaToolResult,
    ) -> TaResult<()> {
        result.content = crate::openhuman::agent::harness::subagent_runner::apply_handoff(
            &self.cache,
            &result.name,
            &self.task_id,
            &self.agent_id,
            std::mem::take(&mut result.content),
        );
        Ok(())
    }
}

/// `before_model` (first call only): "super context" — the graph node analogue
/// of the harness-driven first-turn context collection that used to run
/// imperatively in `session/turn/core.rs`. On the first model call it runs the
/// read-only `context_scout` sub-agent against the raw user ask, folds the
/// resulting `[context_bundle]` into the user message, and registers a
/// prepared-context source so a later `agent_prepare_context` call in the same
/// turn self-suppresses.
///
/// Best-effort: any scout error leaves the turn to proceed with the
/// un-augmented message rather than blocking the user. Runs inside the parent
/// context scope the chat turn already installs (`with_parent_context`), which
/// the scout reads via `current_parent()`.
struct SuperContextMiddleware {
    /// The raw user ask, used as the scout's query (not the enriched message).
    user_message: String,
    /// One-shot latch — `before_model` fires on every model call, but super
    /// context is a first-turn, once-per-run pass.
    ran: AtomicBool,
}

#[async_trait]
impl Middleware<()> for SuperContextMiddleware {
    fn name(&self) -> &str {
        "super_context"
    }

    async fn before_model(
        &self,
        _ctx: &mut RunContext<()>,
        _state: &(),
        request: &mut ModelRequest,
    ) -> TaResult<()> {
        if self.ran.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        let scout = crate::openhuman::agent_orchestration::tools::run_context_scout(
            &self.user_message,
            None,
        )
        .await;
        match scout {
            Ok(result) if !result.is_error => {
                let bundle = result.output();
                // Register the source live so `agent_prepare_context` (which reads
                // `current_agent_context_prepared_sources()`) self-suppresses for
                // the rest of the turn. Only on success — a failed scout must not
                // block a legitimate retry.
                crate::openhuman::agent::harness::push_agent_context_prepared_source(
                    crate::openhuman::agent::harness::AgentContextPreparedSource {
                        source: "super context preparation".to_string(),
                        has_enough_context: parse_context_bundle_has_enough_context(&bundle),
                    },
                );
                tracing::info!(
                    bundle_chars = bundle.chars().count(),
                    "[tinyagents::mw] super_context bundle collected — folding into user message"
                );
                let block = format!(
                    "## Agent context status\n\nAgent context retrieval/preparation has already \
                     run once for this turn in code via super context preparation. Do not call \
                     `agent_prepare_context` again for general context preparation. Use the \
                     prepared context below, and call only specific follow-up tools if a concrete \
                     missing detail is required.\n\n\
                     ## Prepared context (super context)\n\nThe following context was collected \
                     up-front by a read-only context scout before this turn. Use it to ground your \
                     response; do not call `agent_prepare_context` again for general preparation.\n\n\
                     {bundle}\n\n---\n\n"
                );
                prepend_text_to_last_user(&mut request.messages, block);
            }
            Ok(_) => {
                tracing::warn!(
                    "[tinyagents::mw] super_context scout returned an error — proceeding without bundle"
                );
            }
            Err(err) => {
                tracing::warn!(
                    %err,
                    "[tinyagents::mw] super_context collection failed — proceeding without bundle"
                );
            }
        }
        Ok(())
    }
}

/// Prepend a text block to the most recent user message, preserving its existing
/// content blocks (multimodal image blocks survive — the bundle rides in front
/// as a new leading text block). No-op if there is no user message.
fn prepend_text_to_last_user(messages: &mut [TaMessage], block: String) {
    if let Some(TaMessage::User(m)) = messages
        .iter_mut()
        .rev()
        .find(|m| matches!(m, TaMessage::User(_)))
    {
        m.content.insert(0, ContentBlock::Text(block));
    }
}

/// Parse the `has_enough_context: true|false` marker line the context scout
/// emits inside its `[context_bundle]`. Mirrors the former core.rs helper so the
/// prepared-source record carries the same signal. Returns `None` when absent or
/// unparseable.
fn parse_context_bundle_has_enough_context(bundle: &str) -> Option<bool> {
    const PREFIX: &str = "has_enough_context:";
    let line = bundle.lines().map(str::trim).find(|line| {
        line.get(..PREFIX.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(PREFIX))
    })?;
    let value = line[PREFIX.len()..].trim();
    if value.eq_ignore_ascii_case("true") {
        Some(true)
    } else if value.eq_ignore_ascii_case("false") {
        Some(false)
    } else {
        None
    }
}

/// `before_model`: flag volatile tokens (UUIDs, timestamps, JWTs, …) in the
/// system prompt that silently break the provider KV-cache prefix. Warn-only —
/// never mutates the request. The graph analogue of the former
/// `ContextManager::warn_if_cache_unstable`.
struct CacheAlignMiddleware;

#[async_trait]
impl Middleware<()> for CacheAlignMiddleware {
    fn name(&self) -> &str {
        "cache_align"
    }

    async fn before_model(
        &self,
        _ctx: &mut RunContext<()>,
        _state: &(),
        request: &mut ModelRequest,
    ) -> TaResult<()> {
        if let Some(sys) = request
            .messages
            .iter()
            .find(|m| matches!(m, TaMessage::System(_)))
        {
            crate::openhuman::agent::harness::compaction::cache_align::warn_if_volatile(
                &sys.text(),
            );
        }
        Ok(())
    }
}

/// `before_model`: clear the bodies of older tool-result messages, keeping the
/// `keep_recent` most recent verbatim. The graph analogue of
/// `context::microcompact` — bounds a tool-heavy thread's cost without dropping
/// any chat turns. Idempotent: an already-cleared body is left as the
/// placeholder.
struct MicrocompactMiddleware {
    keep_recent: usize,
}

#[async_trait]
impl Middleware<()> for MicrocompactMiddleware {
    fn name(&self) -> &str {
        "microcompact"
    }

    async fn before_model(
        &self,
        _ctx: &mut RunContext<()>,
        _state: &(),
        request: &mut ModelRequest,
    ) -> TaResult<()> {
        let tool_idxs: Vec<usize> = request
            .messages
            .iter()
            .enumerate()
            .filter(|(_, m)| matches!(m, TaMessage::Tool(_)))
            .map(|(i, _)| i)
            .collect();
        if tool_idxs.len() <= self.keep_recent {
            return Ok(());
        }
        let cut = tool_idxs.len() - self.keep_recent;
        for &i in &tool_idxs[..cut] {
            // Skip messages already reduced to the placeholder; otherwise swap the
            // body for it (idempotent, preserves the tool_call_id).
            if request.messages[i].text() == CLEARED_PLACEHOLDER {
                continue;
            }
            if let TaMessage::Tool(t) = &request.messages[i] {
                let id = t.tool_call_id.clone();
                request.messages[i] = TaMessage::tool(id, CLEARED_PLACEHOLDER);
            }
        }
        Ok(())
    }
}

/// `after_tool`: apply the semantic payload summarizer (when configured) and
/// then the hard per-tool-result byte cap to each tool result's model-facing
/// content, before it enters the transcript. The graph analogue of the byte cap
/// + `payload_summarizer` interception the in-house `agent_tool_exec` ran.
struct ToolOutputMiddleware {
    /// Fallback per-tool-result byte cap for tools that don't declare their own.
    budget_bytes: usize,
    payload_summarizer: Option<Arc<dyn PayloadSummarizer>>,
    /// Shared tool sets, used to honor a tool's own `max_result_size_chars()`
    /// cap (issue #4249, Phase 1 Task C) instead of the flat `budget_bytes`.
    tool_sets: Vec<Arc<Vec<Box<dyn Tool>>>>,
}

impl ToolOutputMiddleware {
    /// The tool's own declared character cap, if any.
    fn tool_char_cap(&self, name: &str) -> Option<usize> {
        self.tool_sets
            .iter()
            .flat_map(|set| set.iter())
            .find(|t| t.name() == name)
            .and_then(|t| t.max_result_size_chars())
    }
}

#[async_trait]
impl Middleware<()> for ToolOutputMiddleware {
    fn name(&self) -> &str {
        "tool_output_budget"
    }

    async fn after_tool(
        &self,
        _ctx: &mut RunContext<()>,
        _state: &(),
        result: &mut TaToolResult,
    ) -> TaResult<()> {
        // 1. Semantic summarization (progressive disclosure) — swap the raw
        //    payload for a compressed summary when the summarizer opts in.
        //    Failures never break the tool call (the trait swallows them).
        if let Some(ps) = &self.payload_summarizer {
            if let Ok(Some(payload)) = ps
                .maybe_summarize(&result.name, None, &result.content)
                .await
            {
                tracing::info!(
                    tool = %result.name,
                    from_bytes = payload.original_bytes,
                    to_bytes = payload.summary_bytes,
                    "[tinyagents::mw] payload_summarizer compressed tool output"
                );
                result.content = payload.summary;
            }
        }

        // 2. Per-tool **char** cap — a tool that declares `max_result_size_chars`
        //    caps its own output in characters, with the tool-cap marker the model
        //    was taught to read (legacy engine parity). Distinct from the generic
        //    byte budget below: the tool cap is the tool's own contract.
        let tool_cap = self.tool_char_cap(&result.name);
        if let Some(cap) = tool_cap {
            let char_count = result.content.chars().count();
            if char_count > cap {
                let truncated: String = result.content.chars().take(cap).collect();
                let dropped = char_count - cap;
                tracing::debug!(
                    tool = %result.name,
                    cap,
                    char_count,
                    dropped,
                    "[tinyagents::mw] per-tool char cap applied"
                );
                result.content = format!(
                    "{truncated}\n\n[truncated by tool cap: {dropped} more chars not shown]"
                );
            }
        }

        // 3. Shared byte-cap backstop — truncate at a UTF-8 boundary with a marker.
        //    Only for tools with no cap of their own (a capped tool already bounded
        //    itself above; stacking the two markers would double-truncate).
        if tool_cap.is_none() && self.budget_bytes > 0 {
            let (capped, outcome) =
                apply_tool_result_budget(std::mem::take(&mut result.content), self.budget_bytes);
            if outcome.truncated {
                tracing::debug!(
                    tool = %result.name,
                    from_bytes = outcome.original_bytes,
                    to_bytes = outcome.final_bytes,
                    "[tinyagents::mw] tool_result_budget truncated tool output"
                );
            }
            result.content = capped;
        }
        Ok(())
    }
}

/// `wrap_tool`: route OpenHuman's human-in-the-loop **approval gate** through a
/// named tinyagents tool middleware (issue #4249, Phase 1). A tool with an
/// external effect intercepts through the global [`ApprovalGate`]; a denial
/// short-circuits with the reason as a model-consumable [`TaToolResult`]
/// (`next` is never called), and an allowed call records a terminal audit row
/// once the tool resolves.
///
/// This replaces the inline approval block that used to live in
/// `execute_openhuman_tool`, giving approval a stable middleware name and
/// letting it short-circuit cleanly. Tool-*internal* security (path/command
/// policy via `live_policy`) stays inside each tool — it needs tool-specific
/// operation semantics the harness boundary can't reconstruct generically.
pub struct ApprovalSecurityMiddleware {
    /// The same `Arc`-shared tool sets the runner registers, used to resolve a
    /// call's OpenHuman `Tool` by name so `external_effect_with_args` can gate.
    tool_sets: Vec<Arc<Vec<Box<dyn Tool>>>>,
}

impl ApprovalSecurityMiddleware {
    /// Build the middleware over the runner's shared tool sets.
    pub fn new(tool_sets: Vec<Arc<Vec<Box<dyn Tool>>>>) -> Self {
        Self { tool_sets }
    }

    /// Whether the named tool declares an external effect for these args.
    fn has_external_effect(&self, name: &str, args: &serde_json::Value) -> bool {
        self.tool_sets
            .iter()
            .flat_map(|set| set.iter())
            .find(|t| t.name() == name)
            .map(|t| t.external_effect_with_args(args))
            .unwrap_or(false)
    }
}

#[async_trait]
impl ToolMiddleware<()> for ApprovalSecurityMiddleware {
    fn name(&self) -> &str {
        "approval_security"
    }

    async fn wrap_tool(
        &self,
        ctx: &mut RunContext<()>,
        state: &(),
        call: TaToolCall,
        next: ToolHandler<'_, (), ()>,
    ) -> TaResult<MiddlewareToolOutcome> {
        // Resolve external-effect up front so no tool borrow is held across the
        // approval await.
        let mut audit_id: Option<String> = None;
        if self.has_external_effect(&call.name, &call.arguments) {
            if let Some(gate) = ApprovalGate::try_global() {
                let summary = summarize_action(&call.name, &call.arguments);
                let redacted = redact_args(&call.arguments);
                let (outcome, request_id) =
                    gate.intercept_audited(&call.name, &summary, redacted).await;
                match outcome {
                    GateOutcome::Deny { reason } => {
                        tracing::warn!(
                            tool = %call.name,
                            reason = %reason,
                            "[tinyagents::mw] approval gate denied tool call"
                        );
                        return Ok(MiddlewareToolOutcome::Result(TaToolResult {
                            call_id: call.id,
                            name: call.name,
                            content: reason.clone(),
                            raw: None,
                            error: Some(reason),
                            elapsed_ms: 0,
                        }));
                    }
                    GateOutcome::Allow => audit_id = request_id,
                }
            }
        }

        let outcome = next.run(ctx, state, call).await?;

        // Record the terminal audit row for an approved external-effect call
        // (idempotent; a no-op when the id is unknown).
        if let Some(id) = audit_id {
            if let Some(gate) = ApprovalGate::try_global() {
                if let MiddlewareToolOutcome::Result(res) = &outcome {
                    let exec = if res.error.is_some() {
                        ExecutionOutcome::Failure
                    } else {
                        ExecutionOutcome::Success
                    };
                    gate.record_execution(&id, exec, res.error.as_deref());
                }
            }
        }
        Ok(outcome)
    }
}

/// `wrap_tool`: refuse a tool whose scope is
/// [`ToolScope::CliRpcOnly`](crate::openhuman::tools::ToolScope) inside the
/// autonomous agent loop (issue #4249). The in-house engine ran this gate in
/// `engine::tools`; the tinyagents path dropped it, so a CLI/RPC-only tool
/// (e.g. phone calls) would execute from the model loop. Applies on every path
/// (channel, session, sub-agent) since the restriction is intrinsic to the tool,
/// not the session — installed unconditionally.
pub struct CliRpcOnlyMiddleware {
    tool_sets: Vec<Arc<Vec<Box<dyn Tool>>>>,
}

impl CliRpcOnlyMiddleware {
    pub fn new(tool_sets: Vec<Arc<Vec<Box<dyn Tool>>>>) -> Self {
        Self { tool_sets }
    }

    fn is_cli_rpc_only(&self, name: &str) -> bool {
        self.tool_sets
            .iter()
            .flat_map(|set| set.iter())
            .find(|t| t.name() == name)
            .map(|t| t.scope() == crate::openhuman::tools::ToolScope::CliRpcOnly)
            .unwrap_or(false)
    }
}

#[async_trait]
impl ToolMiddleware<()> for CliRpcOnlyMiddleware {
    fn name(&self) -> &str {
        "cli_rpc_only"
    }

    async fn wrap_tool(
        &self,
        ctx: &mut RunContext<()>,
        state: &(),
        call: TaToolCall,
        next: ToolHandler<'_, (), ()>,
    ) -> TaResult<MiddlewareToolOutcome> {
        if self.is_cli_rpc_only(&call.name) {
            tracing::warn!(
                tool = call.name.as_str(),
                "[tinyagents::mw] tool scope is CliRpcOnly — denied in agent loop"
            );
            let content = format!(
                "Tool '{}' is only available via explicit CLI/RPC invocation, not in the autonomous agent loop.",
                call.name
            );
            return Ok(MiddlewareToolOutcome::Result(TaToolResult {
                call_id: call.id,
                name: call.name,
                content: content.clone(),
                raw: None,
                error: Some(content),
                elapsed_ms: 0,
            }));
        }
        next.run(ctx, state, call).await
    }
}

/// `wrap_tool`: enforce the agent's builder-configured [`ToolPolicy`] at the tool
/// boundary (issue #4249). The in-house engine ran this check in
/// `agent_tool_exec` (`ctx.tool_policy.check(...)`); the tinyagents path bypassed
/// it, so a `.tool_policy()` deny/require-approval silently no-opped and the tool
/// executed anyway — a security regression. This middleware restores it: a
/// blocking decision short-circuits with a model-consumable result carrying the
/// same `"Tool '<name>' <denied|requires approval> by policy '<policy>': <reason>"`
/// wording the engine produced.
pub struct ToolPolicyMiddleware {
    policy: Arc<dyn crate::openhuman::agent::tool_policy::ToolPolicy>,
    /// The session's channel-permission snapshot — enforces the per-channel deny
    /// + per-call permission-level ceiling the engine ran in `agent_tool_exec`.
    session: crate::openhuman::agent_tool_policy::ToolPolicySession,
    /// Shared tool sets (same `Arc`s the runner registers) so a call's OpenHuman
    /// `Tool` can be resolved for its generated-tool runtime context and its
    /// per-call permission level.
    tool_sets: Vec<Arc<Vec<Box<dyn Tool>>>>,
    /// The advertised (visible) tool-name whitelist. Non-empty = restricted; a
    /// call outside it is "not available to this agent" (the engine's first gate).
    /// A non-visible tool is never registered, so it reaches here rewritten onto
    /// the recovery sentinel — its original name rides `requested_tool`.
    visible_tool_names: HashSet<String>,
    session_id: String,
    channel: String,
    agent_definition_id: String,
}

impl ToolPolicyMiddleware {
    pub fn new(
        policy: Arc<dyn crate::openhuman::agent::tool_policy::ToolPolicy>,
        session: crate::openhuman::agent_tool_policy::ToolPolicySession,
        tool_sets: Vec<Arc<Vec<Box<dyn Tool>>>>,
        visible_tool_names: HashSet<String>,
        session_id: String,
        channel: String,
        agent_definition_id: String,
    ) -> Self {
        Self {
            policy,
            session,
            tool_sets,
            visible_tool_names,
            session_id,
            channel,
            agent_definition_id,
        }
    }

    fn resolve_tool(&self, name: &str) -> Option<&Box<dyn Tool>> {
        self.tool_sets
            .iter()
            .flat_map(|set| set.iter())
            .find(|t| t.name() == name)
    }

    /// The channel-permission gate the engine ran before the builder policy: a
    /// session-level deny, then a per-call permission-level ceiling check. Returns
    /// the blocking message when the call must not execute.
    fn channel_permission_block(&self, call: &TaToolCall) -> Option<String> {
        let decision = self.session.decision_for(&call.name);
        if decision.is_denied() {
            let required = decision
                .required_permission
                .map(|permission| permission.to_string())
                .unwrap_or_else(|| "unknown".to_string());
            return Some(format!(
                "Tool '{}' blocked by tool policy: requires {}, channel '{}' allows {}",
                call.name, required, self.channel, decision.allowed_permission
            ));
        }
        let tool = self.resolve_tool(&call.name)?;
        let call_required = tool.permission_level_with_args(&call.arguments);
        if call_required > decision.allowed_permission {
            return Some(format!(
                "Tool '{}' action requires {} permission, channel '{}' allows {}",
                call.name, call_required, self.channel, decision.allowed_permission
            ));
        }
        None
    }

    fn generated_context(
        &self,
        name: &str,
        args: &serde_json::Value,
    ) -> Option<crate::openhuman::agent::tool_policy::GeneratedToolRuntimeContext> {
        self.tool_sets
            .iter()
            .flat_map(|set| set.iter())
            .find(|t| t.name() == name)
            .and_then(|t| t.generated_runtime_context(args))
    }
}

#[async_trait]
impl ToolMiddleware<()> for ToolPolicyMiddleware {
    fn name(&self) -> &str {
        "tool_policy"
    }

    async fn wrap_tool(
        &self,
        ctx: &mut RunContext<()>,
        state: &(),
        call: TaToolCall,
        next: ToolHandler<'_, (), ()>,
    ) -> TaResult<MiddlewareToolOutcome> {
        use crate::openhuman::agent::tool_policy::{
            ToolCallContext, ToolPolicyDecision, ToolPolicyRequest,
        };

        // A call the model made to a tool outside the visible set was registered
        // nowhere, so it arrives rewritten onto the recovery sentinel with the
        // original name on `requested_tool`. When the session restricts visibility
        // (non-empty set) and that original tool isn't visible, the engine's first
        // gate produced "not available to this agent" — reproduce it here (takes
        // precedence over the generic "Unknown tool" sentinel wording). A genuinely
        // unknown call with no visibility restriction still falls through to the
        // sentinel's "Unknown tool" result.
        if call.name == UNKNOWN_TOOL_SENTINEL {
            if !self.visible_tool_names.is_empty() {
                let requested = call
                    .arguments
                    .get("requested_tool")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                if !requested.is_empty() && !self.visible_tool_names.contains(requested) {
                    let content = format!("Tool '{requested}' is not available to this agent");
                    return Ok(MiddlewareToolOutcome::Result(TaToolResult {
                        call_id: call.id,
                        name: call.name,
                        content: content.clone(),
                        raw: None,
                        error: Some(content),
                        elapsed_ms: 0,
                    }));
                }
            }
            return next.run(ctx, state, call).await;
        }

        // Channel-permission ceiling first (session deny + per-call permission
        // level), mirroring the engine order in `agent_tool_exec`.
        if let Some(message) = self.channel_permission_block(&call) {
            tracing::debug!(
                tool = call.name.as_str(),
                channel = self.channel.as_str(),
                "[tinyagents::mw] tool blocked by channel permission ceiling"
            );
            return Ok(MiddlewareToolOutcome::Result(TaToolResult {
                call_id: call.id,
                name: call.name,
                content: message.clone(),
                raw: None,
                error: Some(message),
                elapsed_ms: 0,
            }));
        }

        let context = ToolCallContext::session(
            self.session_id.clone(),
            self.channel.clone(),
            self.agent_definition_id.clone(),
            call.id.clone(),
            1,
        );
        let mut request =
            ToolPolicyRequest::new(call.name.clone(), call.arguments.clone(), context);
        if let Some(generated) = self.generated_context(&call.name, &call.arguments) {
            request = request.with_generated_tool_context(generated);
        }

        let decision = self.policy.check(&request).await;
        if let Some(reason) = decision.blocking_reason() {
            let blocked_action = match &decision {
                ToolPolicyDecision::RequireApproval { .. } => "requires approval",
                ToolPolicyDecision::Deny { .. } => "denied",
                ToolPolicyDecision::Allow => "allowed",
            };
            crate::openhuman::tool_registry::denials::record(
                call.name.as_str(),
                self.policy.name(),
                blocked_action,
                reason,
            );
            tracing::debug!(
                tool = call.name.as_str(),
                policy = self.policy.name(),
                action = blocked_action,
                reason = %reason,
                "[tinyagents::mw] tool blocked by policy"
            );
            let content = format!(
                "Tool '{}' {blocked_action} by policy '{}': {reason}",
                call.name,
                self.policy.name()
            );
            return Ok(MiddlewareToolOutcome::Result(TaToolResult {
                call_id: call.id,
                name: call.name,
                content: content.clone(),
                raw: None,
                error: Some(content),
                elapsed_ms: 0,
            }));
        }

        next.run(ctx, state, call).await
    }
}

/// `before_tool`: rewrite a call to an **unadvertised** tool onto the recovery
/// sentinel (issue #4249, Phase 1 Task B) so a hallucinated tool name is a
/// recoverable [`UnknownToolAdapter`](super::tools::UnknownToolAdapter) result
/// rather than a fatal `ToolNotFound`. `before_tool` runs before the harness
/// resolves the tool, so the rewrite lands in time.
///
/// This moves the decision out of `ProviderModel::response_to_model_response`
/// (which used to carry a `valid_tools` set) to the tool boundary, where it
/// applies uniformly to native and text-parsed tool calls. The sentinel handler
/// is still required (the crate has no "tool not found → repair" hook — SDK gap),
/// but it remains internal and is never advertised to the model.
pub struct UnknownToolRewriteMiddleware {
    /// The set of callable tool names (plus the sentinel). A call outside it is
    /// rewritten onto the sentinel.
    valid: Arc<HashSet<String>>,
}

impl UnknownToolRewriteMiddleware {
    /// Build the middleware over the runner's valid-tool-name set.
    pub fn new(valid: Arc<HashSet<String>>) -> Self {
        Self { valid }
    }
}

#[async_trait]
impl Middleware<()> for UnknownToolRewriteMiddleware {
    fn name(&self) -> &str {
        "unknown_tool_rewrite"
    }

    async fn before_tool(
        &self,
        _ctx: &mut RunContext<()>,
        _state: &(),
        call: &mut TaToolCall,
    ) -> TaResult<()> {
        if call.name != UNKNOWN_TOOL_SENTINEL && !self.valid.contains(&call.name) {
            let requested = std::mem::take(&mut call.name);
            tracing::debug!(
                requested = %requested,
                "[tinyagents::mw] rewriting unknown tool call onto recovery sentinel"
            );
            call.arguments = serde_json::json!({ "requested_tool": requested });
            call.name = UNKNOWN_TOOL_SENTINEL.to_string();
        }
        Ok(())
    }
}

/// `after_tool`: capture each tool call's execution outcome (success + content)
/// into a shared sink before the harness folds the result into a `Message::tool`
/// that drops the `error` flag (issue #4249). Without this, a post-turn
/// `ToolCallRecord` could only report every call as an optimistic success — the
/// in-house engine tracked real per-call success. Runs last in the `after_tool`
/// chain so it records the final (summarized/capped) content the transcript keeps.
pub struct ToolOutcomeCaptureMiddleware {
    sink: super::ToolOutcomeSink,
}

impl ToolOutcomeCaptureMiddleware {
    pub fn new(sink: super::ToolOutcomeSink) -> Self {
        Self { sink }
    }
}

#[async_trait]
impl Middleware<()> for ToolOutcomeCaptureMiddleware {
    fn name(&self) -> &str {
        "tool_outcome_capture"
    }

    async fn after_tool(
        &self,
        _ctx: &mut RunContext<()>,
        _state: &(),
        result: &mut TaToolResult,
    ) -> TaResult<()> {
        if let Ok(mut sink) = self.sink.lock() {
            sink.push(super::ToolCallOutcome {
                call_id: result.call_id.clone(),
                name: result.name.clone(),
                success: result.error.is_none(),
                content: result.content.clone(),
            });
        }
        Ok(())
    }
}

/// `before_tool`: coerce a tool call's arguments to an empty object when they
/// are not a JSON object (issue #4249). A model can emit malformed native
/// arguments (invalid JSON, or a bare scalar/array); the model adapter parses
/// those to `Value::Null`, which the harness then rejects against an object
/// schema and aborts the whole turn. The in-house engine recovered such a call by
/// running the tool with `{}`; restore that so a single bad tool call is
/// recoverable rather than fatal. The recovery sentinel's own
/// `{ "requested_tool": … }` payload is already an object, so it is untouched.
pub struct ArgRecoveryMiddleware;

#[async_trait]
impl Middleware<()> for ArgRecoveryMiddleware {
    fn name(&self) -> &str {
        "arg_recovery"
    }

    async fn before_tool(
        &self,
        _ctx: &mut RunContext<()>,
        _state: &(),
        call: &mut TaToolCall,
    ) -> TaResult<()> {
        if !call.arguments.is_object() {
            tracing::debug!(
                tool = call.name.as_str(),
                "[tinyagents::mw] recovering non-object tool arguments to {{}}"
            );
            call.arguments = serde_json::json!({});
        }
        Ok(())
    }
}

/// `before_model`: enforce OpenHuman's daily/monthly cost budgets **before** a
/// model call spends (issue #4249, Phase 5). Reads the global
/// [`CostTracker`](crate::openhuman::cost) and, when cost budgets are configured
/// and already exceeded, fails the run before the provider call; a warning
/// threshold logs but proceeds.
///
/// Self-gating: a no-op unless a global tracker exists and `config.enabled` with
/// a limit is set (`check_budget` returns `Allowed` otherwise). Complements the
/// post-call `StopHookMiddleware` per-turn USD cap. Projecting the *next* call's
/// cost pre-spend (vs the already-exceeded check here) needs an input-token
/// estimate — a follow-up.
pub struct CostBudgetMiddleware;

#[async_trait]
impl Middleware<()> for CostBudgetMiddleware {
    fn name(&self) -> &str {
        "cost_budget"
    }

    async fn before_model(
        &self,
        _ctx: &mut RunContext<()>,
        _state: &(),
        _request: &mut ModelRequest,
    ) -> TaResult<()> {
        use crate::openhuman::cost::types::BudgetCheck;
        let Some(tracker) = crate::openhuman::cost::try_global() else {
            return Ok(());
        };
        // Pass 0.0 to test whether we are *already* over budget before spending
        // more (rather than projecting this call's cost, which needs a token
        // estimate).
        match tracker.check_budget(0.0) {
            Ok(BudgetCheck::Exceeded {
                current_usd,
                limit_usd,
                period,
            }) => {
                tracing::warn!(
                    %current_usd, %limit_usd, ?period,
                    "[tinyagents::mw] cost budget exceeded — failing before model call"
                );
                Err(tinyagents::TinyAgentsError::LimitExceeded(format!(
                    "cost budget exceeded: {period:?} spend ${current_usd:.4} \u{2265} limit ${limit_usd:.4}"
                )))
            }
            Ok(BudgetCheck::Warning {
                current_usd,
                limit_usd,
                period,
            }) => {
                tracing::warn!(
                    %current_usd, %limit_usd, ?period,
                    "[tinyagents::mw] cost budget warning threshold reached"
                );
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

/// Consecutive **any**-failure no-progress backstop: different commands all
/// failing means the goal is unreachable here. Matches the legacy
/// `NO_PROGRESS_FAILURE_THRESHOLD`.
const NO_PROGRESS_FAILURE_THRESHOLD: usize = 6;
/// Consecutive **identical** hard-policy-rejection repeats before halting — a
/// blocked call re-issued unchanged can never succeed. Legacy
/// `HARD_REJECT_REPEAT_THRESHOLD`.
const HARD_REJECT_REPEAT_THRESHOLD: usize = 2;

/// `after_tool`: stop the run when tool calls keep failing with no progress
/// (issue #4249). The legacy tool loop's progress guard surfaced a root-cause
/// halt summary — a security/approval denial re-issued unchanged, an identical
/// error retried, or *different* commands all failing — instead of burning the
/// whole iteration budget and ending on a generic cap error. The tinyagents path
/// kept only the model/tool call caps, so this reinstates the guard as a graph
/// middleware. Three halt conditions, checked per failure (any success resets
/// every counter — progress was made):
///
/// 1. **Hard policy rejection** (`[policy-blocked]`) repeated `HARD_REJECT_REPEAT_THRESHOLD`
///    times with an identical signature — "blocked by the security policy … re-issued".
/// 2. **Identical** error signature repeated `identical_threshold` times —
///    "retried N times with identical arguments".
/// 3. **Any** failure `NO_PROGRESS_FAILURE_THRESHOLD` times in a row (even with
///    varied errors) — "N tool calls in a row failed".
///
/// On trip it records a root-cause summary into the shared [`HaltSummarySlot`]
/// (the turn overrides its final text with it) and pauses the run via the shared
/// steering handle (same mechanism as the stop-hook / cap pausers).
pub struct RepeatedToolFailureMiddleware {
    handle: SteeringHandle,
    identical_threshold: usize,
    halt_summary: super::HaltSummarySlot,
    state: std::sync::Mutex<FailureState>,
    /// call_id → argument fingerprint, captured in `before_tool` (the tool result
    /// carries no arguments). Folded into the identical-repeat signature so the
    /// "identical arguments" halt only trips on the *same* args — two different
    /// argument sets that happen to share a first error line don't count as a
    /// repeat and can't pre-empt the generic no-progress backstop.
    arg_sigs: std::sync::Mutex<std::collections::HashMap<String, String>>,
}

#[derive(Default)]
struct FailureState {
    last_sig: Option<String>,
    same_count: usize,
    consecutive: usize,
}

impl RepeatedToolFailureMiddleware {
    /// Build the breaker. `identical_threshold` (the identical-signature retry
    /// ceiling) is clamped to at least 2 — a single failure is never a loop.
    pub fn new(
        handle: SteeringHandle,
        identical_threshold: usize,
        halt_summary: super::HaltSummarySlot,
    ) -> Self {
        Self {
            handle,
            identical_threshold: identical_threshold.max(2),
            halt_summary,
            state: std::sync::Mutex::new(FailureState::default()),
            arg_sigs: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }
}

/// A stable, bounded fingerprint of a tool call's arguments for the identical-
/// repeat signature (hashed so a huge payload doesn't bloat the map/comparison).
fn args_fingerprint(arguments: &serde_json::Value) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    arguments.to_string().hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

/// Trim a tool error for inclusion in a halt summary (keep it bounded but retain
/// the deterministic leading detail the model/user needs).
fn truncate_for_halt(text: &str) -> String {
    const MAX: usize = 600;
    crate::openhuman::util::truncate_with_ellipsis(text, MAX)
}

#[async_trait]
impl Middleware<()> for RepeatedToolFailureMiddleware {
    fn name(&self) -> &str {
        "repeated_tool_failure"
    }

    async fn before_tool(
        &self,
        _ctx: &mut RunContext<()>,
        _state: &(),
        call: &mut TaToolCall,
    ) -> TaResult<()> {
        // The tool result carries no arguments, so capture a fingerprint here and
        // correlate it by call_id in `after_tool`.
        if let Ok(mut sigs) = self.arg_sigs.lock() {
            sigs.insert(call.id.clone(), args_fingerprint(&call.arguments));
        }
        Ok(())
    }

    async fn after_tool(
        &self,
        _ctx: &mut RunContext<()>,
        _state: &(),
        result: &mut TaToolResult,
    ) -> TaResult<()> {
        let mut state = self.state.lock().unwrap();
        let arg_fp = self
            .arg_sigs
            .lock()
            .ok()
            .and_then(|mut sigs| sigs.remove(&result.call_id))
            .unwrap_or_default();
        let Some(err) = result.error.as_deref() else {
            // Success → progress was made; reset every counter.
            *state = FailureState::default();
            return Ok(());
        };

        // Signature: tool name + argument fingerprint + first error line (the
        // deterministic parts; a huge payload tail must not dominate the
        // identical-repeat comparison). Including the args means the "identical
        // arguments" halt only fires when the args truly repeat.
        let err_line = err.lines().next().unwrap_or(err);
        let sig = format!("{}\u{1f}{arg_fp}\u{1f}{err_line}", result.name);
        // The unknown-tool recovery is a failure the model can correct (it got the
        // "unknown tool" feedback), so it must NOT feed the generic *any*-failure
        // no-progress counter — else a turn that recovers from one bad tool name
        // and then legitimately exhausts its iteration budget would trip the
        // backstop instead of hitting the cap. It still feeds the *identical*-repeat
        // counter, so re-issuing the SAME unavailable tool halts with a root cause.
        if result.name != UNKNOWN_TOOL_SENTINEL {
            state.consecutive += 1;
        }
        let same_count = match &state.last_sig {
            Some(prev) if *prev == sig => {
                state.same_count += 1;
                state.same_count
            }
            _ => {
                state.last_sig = Some(sig);
                state.same_count = 1;
                1
            }
        };

        // A hard policy rejection is marked in the tool output; it can never
        // succeed when re-issued unchanged, so it trips faster.
        let is_hard_reject = result
            .content
            .contains(crate::openhuman::security::POLICY_BLOCKED_MARKER)
            || err.contains(crate::openhuman::security::POLICY_BLOCKED_MARKER);

        let summary = if is_hard_reject && same_count >= HARD_REJECT_REPEAT_THRESHOLD {
            Some(format!(
                "Stopping: the `{}` call is blocked by the security policy and was re-issued with \
                 identical arguments — it can never succeed this way. Reason:\n{}\n\nDo not repeat \
                 this call; use an allowed alternative or report that it can't be done here.",
                result.name,
                truncate_for_halt(err),
            ))
        } else if same_count >= self.identical_threshold {
            Some(format!(
                "Stopping: the `{}` call was retried {same_count} times with identical arguments \
                 and kept failing — repeating it will not help. Last error:\n{}\n\nThis looks \
                 unrecoverable in the current environment. Report this back instead of retrying.",
                result.name,
                truncate_for_halt(err),
            ))
        } else if state.consecutive >= NO_PROGRESS_FAILURE_THRESHOLD {
            Some(format!(
                "Stopping: {} tool calls in a row failed with no progress. Last error (from \
                 `{}`):\n{}\n\nDifferent commands are all failing — the goal looks unreachable in \
                 this environment. Report this back instead of retrying.",
                state.consecutive,
                result.name,
                truncate_for_halt(err),
            ))
        } else {
            None
        };

        if let Some(summary) = summary {
            tracing::warn!(
                tool = %result.name,
                consecutive = state.consecutive,
                same_count,
                is_hard_reject,
                "[tinyagents::mw] repeated tool failure — halting run so the root cause surfaces"
            );
            if let Ok(mut slot) = self.halt_summary.lock() {
                *slot = Some(summary);
            }
            // Pause at the top of the next iteration (before the next model call),
            // matching the stop-hook / cap pause path. Reset so a resumed run does
            // not immediately re-pause on the same latched state.
            self.handle.send(SteeringCommand::Pause);
            *state = FailureState::default();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tinyagents::harness::context::{RunConfig, RunContext};
    use tinyagents::harness::model::ModelRequest;

    fn ctx() -> RunContext<()> {
        RunContext::new(RunConfig::new("mw-test"), ())
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

    // ── TurnContextMiddleware config ────────────────────────────────────────

    #[test]
    fn defaults_enable_cache_align_and_the_byte_cap_only() {
        let mw = TurnContextMiddleware::defaults();
        assert!(mw.cache_align);
        assert_eq!(
            mw.tool_result_budget_bytes,
            DEFAULT_TOOL_RESULT_BUDGET_BYTES
        );
        assert!(mw.payload_summarizer.is_none());
        assert_eq!(mw.microcompact_keep_recent, 0);
        // Autocompaction defaults on (channel/sub-agent); the chat path overrides
        // it from config.
        assert!(mw.autocompact_enabled);
        assert!(!mw.is_empty());
    }

    #[test]
    fn an_all_default_bundle_installs_nothing() {
        assert!(TurnContextMiddleware::default().is_empty());
    }

    // ── SuperContextMiddleware helpers ──────────────────────────────────────

    #[test]
    fn super_context_is_off_by_default() {
        assert!(TurnContextMiddleware::defaults().super_context.is_none());
        assert!(TurnContextMiddleware::default().super_context.is_none());
    }

    #[test]
    fn parse_bundle_sufficiency_reads_the_marker_case_insensitively() {
        assert_eq!(
            parse_context_bundle_has_enough_context(
                "[context_bundle]\nhas_enough_context: true\n[/context_bundle]"
            ),
            Some(true)
        );
        assert_eq!(
            parse_context_bundle_has_enough_context("HAS_ENOUGH_CONTEXT: false"),
            Some(false)
        );
        assert_eq!(
            parse_context_bundle_has_enough_context("[context_bundle]\nsummary: ok"),
            None
        );
    }

    #[test]
    fn prepend_folds_bundle_ahead_of_the_last_user_message_keeping_images() {
        use tinyagents::harness::message::ImageRef;
        let mut msgs = vec![TaMessage::system("sys"), {
            // A multimodal user turn: text + an image block.
            let mut u = TaMessage::user("original ask");
            if let TaMessage::User(m) = &mut u {
                m.content.push(ContentBlock::Image(ImageRef {
                    url: "data:image/png;base64,AAAA".into(),
                    mime_type: None,
                }));
            }
            u
        }];

        prepend_text_to_last_user(&mut msgs, "BUNDLE_BLOCK\n\n".to_string());

        let TaMessage::User(m) = &msgs[1] else {
            panic!("expected a user message");
        };
        // Bundle rides in front as a new leading text block.
        assert!(
            matches!(&m.content[0], ContentBlock::Text(t) if t.starts_with("BUNDLE_BLOCK")),
            "bundle should be the leading text block"
        );
        // Original text and the image both survive.
        assert!(m
            .content
            .iter()
            .any(|b| matches!(b, ContentBlock::Text(t) if t.contains("original ask"))));
        assert!(
            m.content
                .iter()
                .any(|b| matches!(b, ContentBlock::Image(_))),
            "the image block must survive the fold"
        );
        // System message untouched.
        assert_eq!(msgs[0].text(), "sys");
    }

    #[test]
    fn prepend_is_a_noop_without_a_user_message() {
        let mut msgs = vec![TaMessage::system("only system")];
        prepend_text_to_last_user(&mut msgs, "IGNORED".to_string());
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].text(), "only system");
    }

    // ── MicrocompactMiddleware ──────────────────────────────────────────────

    #[tokio::test]
    async fn microcompact_clears_older_tool_bodies_and_keeps_recent() {
        let mw = MicrocompactMiddleware { keep_recent: 1 };
        let mut req = ModelRequest::new(vec![
            TaMessage::system("sys"),
            TaMessage::user("hello"),
            TaMessage::tool("t1", "FIRST_BODY"),
            TaMessage::assistant("thinking"),
            TaMessage::tool("t2", "SECOND_BODY"),
            TaMessage::tool("t3", "THIRD_BODY"),
        ]);

        mw.before_model(&mut ctx(), &(), &mut req).await.unwrap();

        // 3 tool messages, keep_recent=1 → the two oldest cleared, newest kept.
        assert_eq!(req.messages[2].text(), CLEARED_PLACEHOLDER);
        assert_eq!(req.messages[4].text(), CLEARED_PLACEHOLDER);
        assert_eq!(req.messages[5].text(), "THIRD_BODY");
        // Non-tool messages are never touched.
        assert_eq!(req.messages[0].text(), "sys");
        assert_eq!(req.messages[1].text(), "hello");
        assert_eq!(req.messages[3].text(), "thinking");
    }

    #[tokio::test]
    async fn microcompact_is_a_noop_when_within_keep_recent() {
        let mw = MicrocompactMiddleware { keep_recent: 5 };
        let mut req =
            ModelRequest::new(vec![TaMessage::tool("t1", "A"), TaMessage::tool("t2", "B")]);
        mw.before_model(&mut ctx(), &(), &mut req).await.unwrap();
        assert_eq!(req.messages[0].text(), "A");
        assert_eq!(req.messages[1].text(), "B");
    }

    #[tokio::test]
    async fn microcompact_is_idempotent() {
        let mw = MicrocompactMiddleware { keep_recent: 1 };
        let mut req = ModelRequest::new(vec![
            TaMessage::tool("t1", "FIRST"),
            TaMessage::tool("t2", "SECOND"),
        ]);
        mw.before_model(&mut ctx(), &(), &mut req).await.unwrap();
        let after_first = req.messages[0].text();
        assert_eq!(after_first, CLEARED_PLACEHOLDER);
        // Second pass leaves the already-cleared body as the placeholder.
        mw.before_model(&mut ctx(), &(), &mut req).await.unwrap();
        assert_eq!(req.messages[0].text(), CLEARED_PLACEHOLDER);
        assert_eq!(req.messages[1].text(), "SECOND");
    }

    // ── ToolOutputMiddleware ────────────────────────────────────────────────

    #[tokio::test]
    async fn tool_output_truncates_over_the_flat_budget() {
        let mw = ToolOutputMiddleware {
            budget_bytes: 100,
            payload_summarizer: None,
            tool_sets: vec![],
        };
        let mut result = tool_result("echo", &"x".repeat(5_000));
        mw.after_tool(&mut ctx(), &(), &mut result).await.unwrap();
        assert!(result.content.len() < 5_000, "content should be capped");
        assert!(
            result.content.contains("truncated by tool_result_budget"),
            "a truncation marker should be appended: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn tool_output_leaves_small_results_untouched() {
        let mw = ToolOutputMiddleware {
            budget_bytes: 1_000,
            payload_summarizer: None,
            tool_sets: vec![],
        };
        let mut result = tool_result("echo", "tiny");
        mw.after_tool(&mut ctx(), &(), &mut result).await.unwrap();
        assert_eq!(result.content, "tiny");
    }

    #[test]
    fn tool_char_cap_reads_the_tools_own_declared_cap() {
        let tools: Arc<Vec<Box<dyn Tool>>> = Arc::new(vec![Box::new(FakeTool {
            name: "big",
            cap: Some(10),
            external: false,
        })]);
        let mw = ToolOutputMiddleware {
            budget_bytes: 1_000,
            payload_summarizer: None,
            tool_sets: vec![tools],
        };
        // Tool declares its own char cap → surfaced for the per-tool truncation.
        assert_eq!(mw.tool_char_cap("big"), Some(10));
        // Unknown tool → no per-tool cap (the flat byte budget applies instead).
        assert_eq!(mw.tool_char_cap("other"), None);
    }

    #[tokio::test]
    async fn tool_output_honors_a_tools_own_cap() {
        let tools: Arc<Vec<Box<dyn Tool>>> = Arc::new(vec![Box::new(FakeTool {
            name: "capped",
            cap: Some(20),
            external: false,
        })]);
        let mw = ToolOutputMiddleware {
            budget_bytes: 100_000,
            payload_summarizer: None,
            tool_sets: vec![tools],
        };
        let mut result = tool_result("capped", &"y".repeat(500));
        mw.after_tool(&mut ctx(), &(), &mut result).await.unwrap();
        assert!(
            result
                .content
                .contains("truncated by tool cap: 480 more chars not shown"),
            "the tool's own 20-char cap should truncate with the tool-cap marker: {}",
            result.content
        );
    }

    // ── UnknownToolRewriteMiddleware ────────────────────────────────────────

    #[tokio::test]
    async fn unknown_tool_is_rewritten_onto_the_recovery_sentinel() {
        let valid: Arc<HashSet<String>> = Arc::new(["echo".to_string()].into_iter().collect());
        let mw = UnknownToolRewriteMiddleware::new(valid);
        let mut call = TaToolCall {
            id: "1".into(),
            name: "frobnicate".into(),
            arguments: json!({ "x": 1 }),
        };
        mw.before_tool(&mut ctx(), &(), &mut call).await.unwrap();
        assert_eq!(call.name, UNKNOWN_TOOL_SENTINEL);
        assert_eq!(call.arguments["requested_tool"], json!("frobnicate"));
    }

    #[tokio::test]
    async fn advertised_tool_is_left_untouched() {
        let valid: Arc<HashSet<String>> = Arc::new(["echo".to_string()].into_iter().collect());
        let mw = UnknownToolRewriteMiddleware::new(valid);
        let mut call = TaToolCall {
            id: "1".into(),
            name: "echo".into(),
            arguments: json!({ "msg": "hi" }),
        };
        mw.before_tool(&mut ctx(), &(), &mut call).await.unwrap();
        assert_eq!(call.name, "echo");
        assert_eq!(call.arguments, json!({ "msg": "hi" }));
    }

    #[tokio::test]
    async fn the_sentinel_itself_is_never_rewritten() {
        let valid: Arc<HashSet<String>> = Arc::new(HashSet::new());
        let mw = UnknownToolRewriteMiddleware::new(valid);
        let mut call = TaToolCall {
            id: "1".into(),
            name: UNKNOWN_TOOL_SENTINEL.to_string(),
            arguments: json!({ "requested_tool": "x" }),
        };
        mw.before_tool(&mut ctx(), &(), &mut call).await.unwrap();
        assert_eq!(call.name, UNKNOWN_TOOL_SENTINEL);
    }

    // ── CostBudgetMiddleware ────────────────────────────────────────────────

    #[tokio::test]
    async fn cost_budget_is_a_noop_without_a_global_tracker() {
        // No global CostTracker is installed in the unit-test process, so the
        // gate self-disables and the model call proceeds.
        let mw = CostBudgetMiddleware;
        let mut req = ModelRequest::new(vec![TaMessage::user("hi")]);
        assert!(mw.before_model(&mut ctx(), &(), &mut req).await.is_ok());
    }

    // ── RepeatedToolFailureMiddleware ───────────────────────────────────────

    fn failing_result(name: &str, err: &str) -> TaToolResult {
        let mut r = tool_result(name, err);
        r.error = Some(err.to_string());
        r
    }

    #[tokio::test]
    async fn repeated_tool_failure_pauses_only_after_the_threshold() {
        let handle = SteeringHandle::allow_all();
        let mw = RepeatedToolFailureMiddleware::new(
            handle.clone(),
            3,
            std::sync::Arc::new(std::sync::Mutex::new(None)),
        );
        // Two identical failures: below the threshold, no pause.
        for _ in 0..2 {
            let mut r = failing_result("flaky", "boom");
            mw.after_tool(&mut ctx(), &(), &mut r).await.unwrap();
        }
        assert_eq!(handle.pending(), 0, "no pause before the threshold");
        // Third identical failure trips the breaker.
        let mut r = failing_result("flaky", "boom");
        mw.after_tool(&mut ctx(), &(), &mut r).await.unwrap();
        assert!(
            handle.pending() >= 1,
            "the third identical failure should pause the run"
        );
    }

    #[tokio::test]
    async fn repeated_tool_failure_resets_on_a_success() {
        let handle = SteeringHandle::allow_all();
        let mw = RepeatedToolFailureMiddleware::new(
            handle.clone(),
            3,
            std::sync::Arc::new(std::sync::Mutex::new(None)),
        );
        // Two failures, then a success clears the counter.
        for _ in 0..2 {
            let mut r = failing_result("t", "boom");
            mw.after_tool(&mut ctx(), &(), &mut r).await.unwrap();
        }
        let mut ok = tool_result("t", "fine"); // error = None
        mw.after_tool(&mut ctx(), &(), &mut ok).await.unwrap();
        // Two more failures — still below the threshold because the counter reset.
        for _ in 0..2 {
            let mut r = failing_result("t", "boom");
            mw.after_tool(&mut ctx(), &(), &mut r).await.unwrap();
        }
        assert_eq!(handle.pending(), 0, "a success should reset the breaker");
    }

    #[tokio::test]
    async fn repeated_tool_failure_ignores_distinct_errors() {
        let handle = SteeringHandle::allow_all();
        let mw = RepeatedToolFailureMiddleware::new(
            handle.clone(),
            3,
            std::sync::Arc::new(std::sync::Mutex::new(None)),
        );
        // Three *different* errors never trip the breaker — only an identical,
        // deterministic failure loop does.
        for err in ["e1", "e2", "e3"] {
            let mut r = failing_result("t", err);
            mw.after_tool(&mut ctx(), &(), &mut r).await.unwrap();
        }
        assert_eq!(
            handle.pending(),
            0,
            "distinct errors must not trip the breaker"
        );
    }

    // ── ApprovalSecurityMiddleware ──────────────────────────────────────────

    #[test]
    fn approval_external_effect_resolution_walks_the_tool_sets() {
        let tools: Arc<Vec<Box<dyn Tool>>> = Arc::new(vec![
            Box::new(FakeTool {
                name: "send_email",
                cap: None,
                external: true,
            }),
            Box::new(FakeTool {
                name: "read_file",
                cap: None,
                external: false,
            }),
        ]);
        let mw = ApprovalSecurityMiddleware::new(vec![tools]);
        assert!(mw.has_external_effect("send_email", &json!({})));
        assert!(!mw.has_external_effect("read_file", &json!({})));
        // Unknown tool defaults to no external effect (nothing to gate).
        assert!(!mw.has_external_effect("missing", &json!({})));
    }
}
