//! openhuman context concerns expressed as tinyagents graph middlewares
//! (issue #4249).
//!
//! Historically these ran in the in-house engine's tool/prompt plumbing
//! (`agent_tool_exec`, `ContextManager`). The tinyagents turn path bypassed
//! them, so they were effectively dead on the live loop. Re-expressing them as
//! [`Middleware`] hooks restores the behaviour and makes the graph the single
//! place cross-cutting context concerns live:
//!
//! - [`MicrocompactMiddleware`] (`before_model`) — clear the bodies of older
//!   tool-result messages (keeping the N most recent) so a long tool-heavy
//!   thread stays cheap without dropping chat history. This is now the crate
//!   [`tinyagents::harness::middleware::MicrocompactMiddleware`], constructed
//!   with OpenHuman's [`CLEARED_PLACEHOLDER`] wording; the in-house copy was
//!   upstreamed (see `99-deletion-ledger.md`).
//! - [`ToolOutputMiddleware`] (`after_tool`) — apply the per-tool-result byte
//!   cap and (optionally) the semantic payload summarizer to each tool result
//!   as it returns, before it enters the transcript.
//!
//! [`TurnContextMiddleware`] bundles the config and installs whichever hooks are
//! enabled onto a harness.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use sha2::{Digest, Sha256};

use tinyagents::error::{Result as TaResult, TinyAgentsError};
use tinyagents::harness::context::RunContext;
use tinyagents::harness::events::AgentEvent;
use tinyagents::harness::message::{ContentBlock, Message as TaMessage};
use tinyagents::harness::middleware::{
    AgentRun, BudgetTracker, ContextualToolSelectionMiddleware, MicrocompactMiddleware, Middleware,
    MiddlewareToolOutcome, ToolAllowlistMiddleware, ToolHandler, ToolMiddleware,
};
use tinyagents::harness::model::{ModelRequest, ModelResponse, PromptSegment, SegmentRole};
use tinyagents::harness::no_progress::{NoProgress, NoProgressTracker, ToolAttempt};
use tinyagents::harness::runtime::AgentHarness;
use tinyagents::harness::steering::{SteeringCommand, SteeringHandle};
use tinyagents::harness::tool::{
    ToolCall as TaToolCall, ToolPolicy as TaToolPolicy, ToolResult as TaToolResult, ToolSchema,
};

use crate::openhuman::agent::harness::tool_result_artifacts::{
    apply_per_result_persistence, ToolResultArtifactStore, TINYAGENTS_TOOL_RESULT_ARTIFACT_STORE,
};
use crate::openhuman::approval::{
    redact_args, summarize_action, ApprovalGate, ExecutionOutcome, GateOutcome,
};
use crate::openhuman::context::CLEARED_PLACEHOLDER;
use crate::openhuman::tinyagents::payload_summarizer::PayloadSummarizer;
use crate::openhuman::tokenjuice::AgentTokenjuiceCompression;
use crate::openhuman::tools::Tool;

use super::policy_denial::PolicyDenial;

/// Default per-tool-result byte cap for the channel / sub-agent paths, which do
/// not carry a session `ContextManager` to source the configured budget from.
/// Mirrors the `ContextConfig::tool_result_budget_bytes` default (16 KiB).
const DEFAULT_TOOL_RESULT_BUDGET_BYTES: usize = 16 * 1024;

/// Config bundle for the openhuman context middlewares installed on a turn.
///
/// Cheap to clone (the summarizer is an `Arc`). An all-default value installs
/// nothing — [`install`](Self::install) is a no-op.
#[derive(Clone, Default)]
pub(crate) struct TurnContextMiddleware {
    /// Per-tool-result byte cap. `0` disables the cap.
    pub(crate) tool_result_budget_bytes: usize,
    /// Optional semantic tool-output summarizer (progressive disclosure).
    pub(crate) payload_summarizer: Option<Arc<dyn PayloadSummarizer>>,
    /// Optional action-workspace artifact sink for oversized tool results.
    pub(crate) artifact_store: Option<ToolResultArtifactStore>,
    /// Whether TokenJuice content-aware compaction runs before output caps.
    pub(crate) tokenjuice_compaction_enabled: bool,
    /// Agent-level TokenJuice profile for tool-result compaction.
    pub(crate) tokenjuice_compression: AgentTokenjuiceCompression,
    /// Keep-recent count for microcompact tool-body clearing. `0` disables it.
    pub(crate) microcompact_keep_recent: usize,
    /// Whether the LLM summarization step (`ContextCompressionMiddleware`) may be
    /// installed on this turn. `false` when `[context].enabled` or
    /// `autocompact_enabled` is off, so a diagnostic/test opt-out doesn't spend
    /// summarizer tokens or rewrite history. The deterministic hard-trim backstop
    /// still installs regardless. Defaults to `true` (see [`defaults`](Self::defaults)).
    pub(crate) autocompact_enabled: bool,
    /// "Super context" first-turn context collection. `Some` installs the
    /// [`SuperContextMiddleware`] graph node; `None` (the default, and every
    /// non-chat path) skips it. Only the chat turn sets this — and only when its
    /// gate (`should_run_super_context`) passes.
    pub(crate) super_context: Option<SuperContextConfig>,
    /// Progressive-disclosure handoff: when set (integrations_agent with a
    /// resolved toolkit), oversized tool results are stashed in the shared
    /// [`ResultHandoffCache`] and replaced with an `extract_from_result` drill-in
    /// placeholder. `None` everywhere else.
    pub(crate) handoff: Option<HandoffConfig>,
    /// Live transcript snapshot sink (#4466). When set, a
    /// [`TranscriptSnapshotMiddleware`] mirrors the running conversation (as
    /// openhuman [`ChatMessage`]s) into this shared buffer before every model
    /// call. Only the sub-agent path sets it, so an erroring run can persist the
    /// rounds completed before the failure (the harness drops its partial
    /// transcript on `Err`). `None` everywhere else (chat persists post-run).
    pub(crate) transcript_snapshot: Option<TranscriptSnapshotSink>,
}

/// Shared buffer a [`TranscriptSnapshotMiddleware`] mirrors the live sub-agent
/// conversation into, so the caller can persist completed rounds even when the
/// harness run ends in `Err` (#4466).
pub(crate) type TranscriptSnapshotSink =
    Arc<std::sync::Mutex<Vec<crate::openhuman::inference::provider::ChatMessage>>>;

/// Observation-only middleware that snapshots the running transcript into a
/// shared [`TranscriptSnapshotSink`] before each model call (#4466).
///
/// The tinyagents harness owns the working message vector and only hands it back
/// inside a successful `AgentRun`; on a mid-run error it is dropped. The
/// sub-agent runner persists a per-child `session_raw` transcript so
/// `learning/transcript_ingest` can read it — but a failed run used to persist
/// nothing. This middleware mirrors each `before_model` request's messages
/// (which include every prior completed assistant/tool round) into an
/// openhuman-owned buffer, so the runner's error path can still write the rounds
/// that completed before the failure. Converts to [`ChatMessage`] eagerly so the
/// caller does not need access to the private `convert` module.
pub(crate) struct TranscriptSnapshotMiddleware {
    sink: TranscriptSnapshotSink,
}

#[async_trait]
impl Middleware<()> for TranscriptSnapshotMiddleware {
    fn name(&self) -> &str {
        "openhuman.transcript_snapshot"
    }

    async fn before_model(
        &self,
        _ctx: &mut RunContext<()>,
        _state: &(),
        request: &mut ModelRequest,
    ) -> TaResult<()> {
        let history = super::convert::messages_to_history(&request.messages);
        if let Ok(mut guard) = self.sink.lock() {
            *guard = history;
        }
        Ok(())
    }
}

/// Config for the [`HandoffMiddleware`]: the per-spawn cache (shared with the
/// `extract_from_result` tool) plus the ids used in handoff log lines.
#[derive(Clone)]
pub(crate) struct HandoffConfig {
    pub(crate) cache: Arc<crate::openhuman::agent::harness::subagent_runner::ResultHandoffCache>,
    pub(crate) agent_id: String,
    pub(crate) task_id: String,
}

/// SHADOW tool-exposure middleware (issue #4249, 01.3 — dynamic exposure).
///
/// This is the **adapter-first landing** of the crate-native tool-selection
/// layer. It expresses OpenHuman's exposure policy (agent
/// `tool_allowlist`/`tool_denylist` + sub-agent scope + MCP visibility + channel
/// permission ceiling — all already collapsed by the precompute path into the
/// single `allowed` visible set handed to `assemble_turn_harness`) as a composed
/// crate selection layer:
///
/// - a [`ToolAllowlistMiddleware`] for the static allow guard, and
/// - one [`ContextualToolSelectionMiddleware`] built via
///   [`ContextualToolSelectionMiddleware::inheriting`] so a delegated child can
///   only ever *narrow* the parent's exposure (sub-agent-cannot-exceed-parent).
///
/// It runs in **SHADOW**: on the first model call it drives the composed crate
/// selection over a scratch [`ModelRequest`] built from the **broad candidate
/// set** (not the live request, whose `tools` OpenHuman already narrowed), so it
/// (a) makes the exposure decision **event-native** via the crate selection's own
/// [`AgentEvent::ToolsFiltered`] emit, and (b) logs any DIVERGENCE (grep-friendly
/// `[tool-exposure]`) between what the crate layer would expose and the set
/// OpenHuman actually registered as callable. It **never** mutates the live
/// `ModelRequest::tools`, so the model's actually-callable tool set is
/// byte-identical to today (zero behavior risk). Exposure is fail-closed in the
/// COMPUTATION (a candidate absent from `allowed` is excluded), but that decision
/// is only logged/emitted — not enforced — this slice.
///
/// Ownership flip (making this crate selection the sole authority + deleting
/// `agent/harness/tool_filter.rs` and `subagent_runner/tool_prep.rs`) is the
/// GATED follow-up, once the `[tool-exposure]` divergence logs show parity.
pub(super) struct OpenHumanToolExposureShadowMiddleware {
    /// Static allow guard (crate). Held for the fail-closed parity cross-check;
    /// NOT installed as a live `before_tool` execution guard this slice —
    /// OpenHuman already registers only the `allowed` set, so the model can never
    /// call a hidden tool.
    allowlist: ToolAllowlistMiddleware,
    /// The composed contextual selection layer, built via `inheriting(...)`:
    /// parent ceiling = broad candidate set, child = precomputed visible set. Its
    /// `before_model` drives the shadow retain + emits `ToolsFiltered`.
    selection: ContextualToolSelectionMiddleware,
    /// Broad candidate tool set (names before the precompute narrowed it), as
    /// scratch schemas the shadow selection filters over.
    candidates: Vec<ToolSchema>,
    /// The set OpenHuman actually registered as callable this turn — the
    /// divergence reference.
    registered: std::collections::HashSet<String>,
    /// agent id / task kind / security tier / channel encoded as selection tags
    /// (carried onto the scratch request + surfaced in the divergence log). The
    /// `inheriting`/`from_lists` predicate is name-based today, so these tags are
    /// documentary context for the ownership-flip follow-up.
    tags: Vec<String>,
    /// One-shot latch — `before_model` fires on every model call, but the shadow
    /// exposure decision is a once-per-run computation.
    ran: AtomicBool,
}

impl OpenHumanToolExposureShadowMiddleware {
    /// Build the shadow layer from the SAME inputs the precompute path feeds the
    /// runner: the broad `candidate_names` and the narrowed `allowed` visible set.
    /// Allowlist semantics are **fail-closed** (issue #4452): `None` means "no
    /// filter supplied → all candidates visible"; `Some(set)` means "exactly the
    /// named tools", so `Some(empty)` is a genuine deny-all. This mirrors the
    /// registration loop in `assemble_turn_harness`, keeping the shadow divergence
    /// reference in step with what OpenHuman actually registers as callable.
    pub(super) fn new(
        candidate_names: &[String],
        allowed: Option<&std::collections::HashSet<String>>,
        tags: Vec<String>,
    ) -> Self {
        // Effective visible set: `None` → every candidate; `Some(set)` → exactly
        // the candidates named in `set` (empty set → none). Fail-closed: a
        // candidate absent from a supplied `allowed` is excluded (not exposed).
        let registered: std::collections::HashSet<String> = match allowed {
            None => candidate_names.iter().cloned().collect(),
            Some(set) => candidate_names
                .iter()
                .filter(|name| set.contains(*name))
                .cloned()
                .collect(),
        };
        let excluded: Vec<String> = candidate_names
            .iter()
            .filter(|name| !registered.contains(*name))
            .cloned()
            .collect();
        // Compose the crate selection via `inheriting` so a child can only narrow:
        // parent ceiling = the broad candidate set (deny none), child = the
        // precomputed visible set (deny the withheld candidates). The effective
        // allow is `candidates ∩ registered == registered ⊆ candidates`, so the
        // decision can never widen beyond what the parent candidate context could
        // grant — the sub-agent-cannot-exceed-parent invariant, computed.
        let selection = ContextualToolSelectionMiddleware::inheriting(
            Some(candidate_names.to_vec()),
            Vec::<String>::new(),
            Some(registered.iter().cloned().collect::<Vec<_>>()),
            excluded,
        );
        let allowlist = ToolAllowlistMiddleware::new(registered.iter().cloned());
        let candidates = candidate_names
            .iter()
            .map(|name| ToolSchema::new(name.clone(), String::new(), serde_json::json!({})))
            .collect();
        Self {
            allowlist,
            selection,
            candidates,
            registered,
            tags,
            ran: AtomicBool::new(false),
        }
    }
}

/// Inputs the [`SuperContextMiddleware`] node needs to run its first-turn
/// read-only context-collection pass.
#[derive(Clone)]
pub(crate) struct SuperContextConfig {
    /// The raw user ask, used as the context scout's query.
    pub(crate) user_message: String,
}

impl TurnContextMiddleware {
    /// A sensible default for turn paths without a session `ContextManager`
    /// (channel / sub-agent): the default tool-result byte cap, no summarizer or
    /// microcompact.
    pub(crate) fn defaults() -> Self {
        Self {
            tool_result_budget_bytes: DEFAULT_TOOL_RESULT_BUDGET_BYTES,
            payload_summarizer: None,
            artifact_store: None,
            tokenjuice_compaction_enabled: false,
            tokenjuice_compression: AgentTokenjuiceCompression::Off,
            microcompact_keep_recent: 0,
            autocompact_enabled: true,
            super_context: None,
            handoff: None,
            transcript_snapshot: None,
        }
    }

    /// `true` when no middleware would be installed.
    pub(crate) fn is_empty(&self) -> bool {
        self.tool_result_budget_bytes == 0
            && self.payload_summarizer.is_none()
            && !self.tokenjuice_compaction_enabled
            && self.microcompact_keep_recent == 0
            && self.super_context.is_none()
            && self.handoff.is_none()
            && self.transcript_snapshot.is_none()
    }

    /// Push the enabled middlewares onto `harness`.
    ///
    /// `before_model` hooks run in registration order, so microcompact (clear
    /// tool bodies) is installed **before** the caller's summarization / trim
    /// middlewares — microcompact frees cheap tokens first, then
    /// summarization/trim handle the rest.
    pub(crate) fn install(
        self,
        harness: &mut AgentHarness<()>,
        tool_policies: HashMap<String, TaToolPolicy>,
    ) {
        // Transcript snapshot (#4466) runs first among before_model hooks so it
        // mirrors the *incoming* request transcript (every prior completed round)
        // before microcompact/summarization rewrite it — the caller's error path
        // persists exactly what the model was about to see.
        if let Some(sink) = self.transcript_snapshot {
            harness.push_middleware(Arc::new(TranscriptSnapshotMiddleware { sink }));
        }
        // Super context runs first: it prepares the read-only context bundle and
        // folds it into the first model call's user message before any other
        // before_model hook inspects the request.
        if let Some(sc) = self.super_context {
            harness.push_middleware(Arc::new(SuperContextMiddleware {
                user_message: sc.user_message,
                ran: AtomicBool::new(false),
            }));
        }
        if self.microcompact_keep_recent > 0 {
            // Crate middleware (upstreamed from the in-house copy). Constructed
            // with OpenHuman's model-facing placeholder so behavior is
            // byte-identical to the deleted local version. Events stay off (the
            // default) to preserve the prior silent-rewrite behavior.
            harness.push_middleware(Arc::new(MicrocompactMiddleware::new(
                self.microcompact_keep_recent,
                CLEARED_PLACEHOLDER,
            )));
        }
        // REVERSE-ORDER RULE (issue #4464): the crate runs `after_tool` hooks in
        // REVERSE registration order (`MiddlewareStack::run_after_tool` iterates
        // `self.middlewares.iter().rev()`, tinyagents src/harness/middleware/mod.rs).
        // So the LAST-pushed middleware's `after_tool` runs FIRST. To make the
        // effective `after_tool` chain be handoff(raw) → tool-output budget/caps,
        // the handoff MUST be pushed AFTER the tool-output budget.
        //
        // Push the tool-output budget FIRST (so its `after_tool` runs SECOND):
        // it truncates the oversized payload to the 16 KiB byte cap.
        if self.tool_result_budget_bytes > 0
            || self.payload_summarizer.is_some()
            || self.tokenjuice_compaction_enabled
        {
            harness.push_middleware(Arc::new(ToolOutputMiddleware {
                budget_bytes: self.tool_result_budget_bytes,
                payload_summarizer: self.payload_summarizer,
                artifact_store: self.artifact_store,
                tokenjuice_compaction_enabled: self.tokenjuice_compaction_enabled,
                tokenjuice_compression: self.tokenjuice_compression,
                tool_policies,
            }));
        }
        // Push the handoff LAST (so its `after_tool` runs FIRST): it observes the
        // RAW, uncapped payload, stashes an oversized result into the
        // `ResultHandoffCache`, and swaps in a short pointer BEFORE the tool-output
        // budget can shrink it below the 50k-token handoff threshold and defeat the
        // drill-in.
        if let Some(handoff) = self.handoff {
            harness.push_middleware(Arc::new(HandoffMiddleware {
                cache: handoff.cache,
                agent_id: handoff.agent_id,
                task_id: handoff.task_id,
            }));
        }
    }
}

fn estimate_output_tokens(bytes: usize) -> u64 {
    bytes.div_ceil(4) as u64
}

#[async_trait]
impl Middleware<()> for OpenHumanToolExposureShadowMiddleware {
    fn name(&self) -> &str {
        "openhuman_tool_exposure_shadow"
    }

    async fn before_model(
        &self,
        ctx: &mut RunContext<()>,
        state: &(),
        request: &mut ModelRequest,
    ) -> TaResult<()> {
        // Once-per-run: the exposure decision is stable for the turn.
        if self.ran.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        // SHADOW: drive the composed crate selection over a SCRATCH request built
        // from the BROAD candidate set — deliberately NOT the live `request`,
        // whose `tools` OpenHuman already narrowed to the visible set. This lets
        // the crate layer compute the exposure decision over the full candidate
        // context and emit it event-native (the crate
        // `ContextualToolSelectionMiddleware::before_model` emits
        // `AgentEvent::ToolsFiltered` on `ctx` for the withheld candidates) —
        // without ever dropping a tool the model can actually call. The live
        // `request.tools` is left untouched.
        let mut scratch = ModelRequest {
            tools: self.candidates.clone(),
            model: request.model.clone(),
            tags: self.tags.clone(),
            ..Default::default()
        };
        // Reuse the crate selection's own retain + `ToolsFiltered` emit verbatim.
        self.selection
            .before_model(ctx, state, &mut scratch)
            .await?;
        let shadow_exposed: std::collections::HashSet<String> =
            scratch.tools.iter().map(|s| s.name.clone()).collect();

        // Divergence vs what OpenHuman actually registered as callable this turn.
        let mut missing_from_shadow: Vec<&String> = self
            .registered
            .iter()
            .filter(|name| !shadow_exposed.contains(*name))
            .collect();
        let mut extra_in_shadow: Vec<&String> = shadow_exposed
            .iter()
            .filter(|name| !self.registered.contains(*name))
            .collect();
        // Fail-closed cross-check: every shadow-exposed name must also pass the
        // static allow guard (they are built from the same set, so this should be
        // vacuously true; a mismatch would flag a policy-composition bug).
        let mut allowlist_disagree: Vec<&String> = shadow_exposed
            .iter()
            .filter(|name| !self.allowlist.allows(name))
            .collect();
        missing_from_shadow.sort();
        extra_in_shadow.sort();
        allowlist_disagree.sort();

        if missing_from_shadow.is_empty()
            && extra_in_shadow.is_empty()
            && allowlist_disagree.is_empty()
        {
            tracing::debug!(
                exposed = shadow_exposed.len(),
                candidates = self.candidates.len(),
                registered = self.registered.len(),
                tags = ?self.tags,
                "[tool-exposure] shadow crate selection agrees with OpenHuman precompute (parity)"
            );
        } else {
            tracing::warn!(
                ?missing_from_shadow,
                ?extra_in_shadow,
                ?allowlist_disagree,
                registered = self.registered.len(),
                shadow_exposed = shadow_exposed.len(),
                candidates = self.candidates.len(),
                tags = ?self.tags,
                "[tool-exposure] DIVERGENCE: shadow crate selection differs from OpenHuman precompute — NOT enforced (SHADOW; ownership flip is the gated follow-up)"
            );
        }
        Ok(())
    }
}

/// `after_tool`: progressive-disclosure handoff (issue #4249 1b). An oversized
/// sub-agent tool result is stashed in the shared [`ResultHandoffCache`] and its
/// content replaced with a short placeholder naming a `result_id` the model can
/// drill into via `extract_from_result`. Restores the seam the legacy
/// `SubagentToolSource` ran on every tool result (via `apply_handoff`), which the
/// agent_graph rewrite dropped. Errors and `extract_from_result`'s own output
/// pass through unchanged (handled inside `apply_handoff`).
pub(crate) struct HandoffMiddleware {
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

/// Stable SHA-256 fingerprint over canonical JSON. TinyAgents' prompt builder
/// uses the same shape for `ModelRequest::prompt_fingerprint`; OpenHuman builds
/// requests directly, so this adapter must stamp equivalent content-derived
/// segment ids and request fingerprints.
fn stable_prefix_fingerprint(value: &serde_json::Value) -> String {
    let mut hasher = Sha256::new();
    if serde_json::to_writer(&mut hasher, value).is_err() {
        hasher = Sha256::new();
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// `before_model`: declare the turn's stable prompt prefix (system prompt + tool
/// schemas) as [`PromptSegment`]s on the [`ModelRequest`] (issue #4249, 03.2).
///
/// OpenHuman assembles the request's messages/tools directly rather than through
/// the crate prompt builder, so `cache_segments` would otherwise stay empty and
/// the crate `PromptCacheGuardMiddleware` (installed immediately after this)
/// would have no prefix to protect. This stamps the segments with
/// **content-fingerprint ids**: an unchanged system prompt + full tool-schema set
/// yields a stable prefix, while an injected timestamp/uuid/etc. or changed tool
/// schema flips it and the guard records a
/// [`CacheLayoutEvent`](tinyagents::harness::cache::CacheLayoutEvent). This is
/// the structured, crate-native replacement for the deleted warn-only
/// `CacheAlignMiddleware` volatile-token scan (C3): the crate
/// `PromptCacheGuardMiddleware` now owns KV-cache-prefix drift detection via
/// recorded `CacheLayoutEvent`s. Read-only w.r.t. the transcript — only sets
/// `cache_segments` / `prompt_fingerprint`.
pub(crate) struct PromptCacheSegmentMiddleware;

#[async_trait]
impl Middleware<()> for PromptCacheSegmentMiddleware {
    fn name(&self) -> &str {
        "prompt_cache_segments"
    }

    async fn before_model(
        &self,
        _ctx: &mut RunContext<()>,
        _state: &(),
        request: &mut ModelRequest,
    ) -> TaResult<()> {
        let mut segments: Vec<PromptSegment> = Vec::new();
        // 1. System prompt — the cache-hottest stable prefix segment.
        if let Some(sys) = request
            .messages
            .iter()
            .find(|m| matches!(m, TaMessage::System(_)))
        {
            let fp = stable_prefix_fingerprint(&serde_json::json!({
                "role": "system",
                "messages": [sys],
            }));
            segments.push(PromptSegment {
                id: format!("system:{fp}"),
                role: SegmentRole::System,
                cacheable: true,
            });
        }
        // 2. Tool schemas — advertised tool surface identity (full schemas, in
        //    registration order) forms the next stable prefix segment. A changed
        //    tool surface legitimately busts the prefix; an unchanged one keeps
        //    it stable.
        if !request.tools.is_empty() {
            let fp = stable_prefix_fingerprint(&serde_json::json!({
                "role": "tools",
                "tools": &request.tools,
            }));
            segments.push(PromptSegment {
                id: format!("tools:{fp}"),
                role: SegmentRole::Tools,
                cacheable: true,
            });
        }
        if !segments.is_empty() {
            request.prompt_fingerprint = Some(stable_prefix_fingerprint(&serde_json::json!({
                "segments": &segments,
                "tools": &request.tools,
            })));
            tracing::debug!(
                segment_count = segments.len(),
                fingerprint = request.prompt_fingerprint.as_deref().unwrap_or(""),
                "[cache] declared stable prompt-prefix segments for KV-cache guard"
            );
            request.cache_segments = segments;
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
    artifact_store: Option<ToolResultArtifactStore>,
    tokenjuice_compaction_enabled: bool,
    tokenjuice_compression: AgentTokenjuiceCompression,
    /// SDK policy snapshot keyed by tool name. Used to honor the adapter-mapped
    /// `max_result_size_chars()` cap without re-querying the OpenHuman tool
    /// trait from `after_tool`.
    tool_policies: HashMap<String, TaToolPolicy>,
}

impl ToolOutputMiddleware {
    /// The tool's own declared cap, if any. The adapter maps OpenHuman's
    /// `max_result_size_chars()` into `ToolRuntime.max_result_bytes`; preserving
    /// char-based truncation here keeps the existing model-facing marker stable.
    fn tool_char_cap(&self, name: &str) -> Option<usize> {
        self.tool_policies
            .get(name)
            .and_then(|policy| policy.runtime.max_result_bytes)
    }
}

#[async_trait]
impl Middleware<()> for ToolOutputMiddleware {
    fn name(&self) -> &str {
        "tool_output_budget"
    }

    async fn after_tool(
        &self,
        ctx: &mut RunContext<()>,
        _state: &(),
        result: &mut TaToolResult,
    ) -> TaResult<()> {
        // 1. Semantic summarization (progressive disclosure) — swap the raw
        //    payload for a compressed summary when the summarizer opts in.
        //    Failures never break the tool call (the trait swallows them).
        if let Some(ps) = &self.payload_summarizer {
            if let Ok(Some(payload)) = ps
                .maybe_summarize_in_parent(ctx, &result.name, None, &result.content)
                .await
            {
                tracing::info!(
                    tool = %result.name,
                    from_bytes = payload.original_bytes,
                    to_bytes = payload.summary_bytes,
                    "[tinyagents::mw] payload_summarizer compressed tool output"
                );
                ctx.emit(AgentEvent::Compressed {
                    from_tokens: estimate_output_tokens(payload.original_bytes),
                    to_tokens: estimate_output_tokens(payload.summary_bytes),
                });
                result.content = payload.summary;
            }
        }

        // 2. TokenJuice content-aware compaction. This mirrors the legacy
        //    `agent_tool_exec` stage that ran after semantic summarization and
        //    before the hard output caps.
        let before_tokenjuice_bytes = result.content.len();
        let compacted = crate::openhuman::tokenjuice::compact_output_with_policy(
            std::mem::take(&mut result.content),
            &result.name,
            self.tokenjuice_compaction_enabled,
            self.tokenjuice_compression,
        )
        .await;
        result.content = compacted;
        let after_tokenjuice_bytes = result.content.len();
        if after_tokenjuice_bytes < before_tokenjuice_bytes {
            ctx.emit(AgentEvent::Compressed {
                from_tokens: estimate_output_tokens(before_tokenjuice_bytes),
                to_tokens: estimate_output_tokens(after_tokenjuice_bytes),
            });
        }

        // 3. Per-tool **char** cap — a tool that declares `max_result_size_chars`
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

        // 4. Shared byte-cap backstop — truncate at a UTF-8 boundary with a marker.
        //    Only for tools with no cap of their own (a capped tool already bounded
        //    itself above; stacking the two markers would double-truncate).
        if tool_cap.is_none() && self.budget_bytes > 0 {
            let (capped, outcome) = apply_per_result_persistence(
                std::mem::take(&mut result.content),
                self.artifact_store.as_ref(),
                &result.name,
                Some(&result.call_id),
                self.budget_bytes,
            )
            .await;
            if outcome.persisted {
                tracing::info!(
                    tool = %result.name,
                    from_bytes = outcome.original_bytes,
                    to_bytes = outcome.final_bytes,
                    "[tinyagents::mw] tool_result_artifact persisted oversized output"
                );
                if let Some(path) = outcome.artifact_path.as_deref() {
                    if let Some(store) = ctx.stores.get(TINYAGENTS_TOOL_RESULT_ARTIFACT_STORE) {
                        let key = result.call_id.clone();
                        let mut fields = serde_json::Map::new();
                        fields.insert("tool".to_string(), result.name.clone().into());
                        fields.insert("call_id".to_string(), result.call_id.clone().into());
                        fields.insert("artifact_path".to_string(), path.to_string().into());
                        fields.insert(
                            "original_bytes".to_string(),
                            serde_json::Value::from(outcome.original_bytes as u64),
                        );
                        fields.insert(
                            "preview_bytes".to_string(),
                            serde_json::Value::from(outcome.final_bytes as u64),
                        );
                        let index_result: tinyagents::Result<()> =
                            store.put("tool_results", &key, fields.into()).await;
                        if let Err(err) = index_result {
                            tracing::warn!(
                                tool = %result.name,
                                call_id = %result.call_id,
                                error = %err,
                                "[tinyagents::mw] failed to index tool_result_artifact"
                            );
                        } else {
                            tracing::debug!(
                                tool = %result.name,
                                call_id = %result.call_id,
                                artifact_path = %path,
                                "[tinyagents::mw] indexed tool_result_artifact in run store"
                            );
                        }
                    }
                }
            } else if outcome.original_bytes != outcome.final_bytes {
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
pub(super) struct ApprovalSecurityMiddleware {
    /// The same `Arc`-shared tool sets the runner registers, used to resolve a
    /// call's OpenHuman `Tool` by name so `external_effect_with_args` can gate.
    tool_sets: Vec<Arc<Vec<Box<dyn Tool>>>>,
}

impl ApprovalSecurityMiddleware {
    /// Build the middleware over the runner's shared tool sets.
    pub(super) fn new(tool_sets: Vec<Arc<Vec<Box<dyn Tool>>>>) -> Self {
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
pub(super) struct CliRpcOnlyMiddleware {
    tool_sets: Vec<Arc<Vec<Box<dyn Tool>>>>,
}

impl CliRpcOnlyMiddleware {
    pub(super) fn new(tool_sets: Vec<Arc<Vec<Box<dyn Tool>>>>) -> Self {
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

/// `wrap_tool`: scrub credential-shaped secrets out of every tool result before
/// it leaves the tool boundary (issue #4453). The legacy engine ran
/// `scrub_credentials` over **every** tool output before it entered model
/// context (`engine/tools.rs`); the tinyagents path dropped that call site, so
/// secrets in tool output (env dumps, config reads, API responses, shell output)
/// reached model context, on-disk `session_raw` transcripts, worker-thread
/// mirrors, and the tool-outcome capture sink — violating "Never log secrets or
/// full PII".
///
/// Installed as the **innermost** tool wrap (pushed last), so it observes the
/// RAW tool result first and scrubs it before any outer wrap, the `after_tool`
/// chain (summarization/caps in [`ToolOutputMiddleware`]), the transcript push,
/// or the [`ToolOutcomeCaptureMiddleware`] sink can see the unredacted content.
/// Scrubbing here — rather than inside `execute_openhuman_tool` — covers the
/// parent chat path, sub-agent paths, the persisted transcript, and
/// `ToolCallOutcome` records by construction, since every path runs the same
/// `assemble_turn_harness` seam.
pub(super) struct CredentialScrubMiddleware;

impl CredentialScrubMiddleware {
    pub(super) fn new() -> Self {
        Self
    }
}

#[async_trait]
impl ToolMiddleware<()> for CredentialScrubMiddleware {
    fn name(&self) -> &str {
        "credential_scrub"
    }

    async fn wrap_tool(
        &self,
        ctx: &mut RunContext<()>,
        state: &(),
        call: TaToolCall,
        next: ToolHandler<'_, (), ()>,
    ) -> TaResult<MiddlewareToolOutcome> {
        let tool_name = call.name.clone();
        let outcome = next.run(ctx, state, call).await?;
        // `MiddlewareToolOutcome` is `#[non_exhaustive]`; today it only carries a
        // `Result`, but match rather than irrefutable-let so a future variant
        // fails loud instead of silently bypassing scrubbing.
        let mut result = match outcome {
            MiddlewareToolOutcome::Result(result) => result,
            other => return Ok(other),
        };

        let scrubbed_content =
            crate::openhuman::agent::harness::credentials::scrub_credentials(&result.content);
        if scrubbed_content != result.content {
            tracing::warn!(
                tool = %tool_name,
                "[tinyagents::mw] credential_scrub redacted secret(s) from tool result content"
            );
            result.content = scrubbed_content;
        }

        if let Some(err) = result.error.as_ref() {
            let scrubbed_err =
                crate::openhuman::agent::harness::credentials::scrub_credentials(err);
            if &scrubbed_err != err {
                tracing::warn!(
                    tool = %tool_name,
                    "[tinyagents::mw] credential_scrub redacted secret(s) from tool result error"
                );
                result.error = Some(scrubbed_err);
            }
        }

        // Raw JSON payloads (rarely populated on this path) can carry the same
        // secrets — walk their string leaves so a scrubbed `content` isn't
        // undermined by an unredacted `raw` mirror.
        if let Some(raw) = result.raw.take() {
            result.raw = Some(scrub_json_credentials(raw));
        }

        Ok(MiddlewareToolOutcome::Result(result))
    }
}

/// Recursively scrub credential-shaped string leaves inside a JSON value.
fn scrub_json_credentials(value: serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match value {
        Value::String(s) => {
            Value::String(crate::openhuman::agent::harness::credentials::scrub_credentials(&s))
        }
        Value::Array(items) => {
            Value::Array(items.into_iter().map(scrub_json_credentials).collect())
        }
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(k, v)| (k, scrub_json_credentials(v)))
                .collect(),
        ),
        other => other,
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
pub(super) struct ToolPolicyMiddleware {
    policy: Arc<dyn crate::openhuman::agent::tool_policy::ToolPolicy>,
    /// The session's channel-permission snapshot — enforces the per-channel deny
    /// + per-call permission-level ceiling the engine ran in `agent_tool_exec`.
    session: crate::openhuman::agent_tool_policy::ToolPolicySession,
    /// Shared tool sets (same `Arc`s the runner registers) so a call's OpenHuman
    /// `Tool` can be resolved for its generated-tool runtime context and its
    /// per-call permission level.
    tool_sets: Vec<Arc<Vec<Box<dyn Tool>>>>,
    session_id: String,
    channel: String,
    agent_definition_id: String,
}

impl ToolPolicyMiddleware {
    pub(super) fn new(
        policy: Arc<dyn crate::openhuman::agent::tool_policy::ToolPolicy>,
        session: crate::openhuman::agent_tool_policy::ToolPolicySession,
        tool_sets: Vec<Arc<Vec<Box<dyn Tool>>>>,
        session_id: String,
        channel: String,
        agent_definition_id: String,
    ) -> Self {
        Self {
            policy,
            session,
            tool_sets,
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
            return Some(
                PolicyDenial::SessionForbidden {
                    tool: &call.name,
                    required: decision.required_permission,
                    allowed: decision.allowed_permission,
                    channel: &self.channel,
                }
                .render(),
            );
        }
        let tool = self.resolve_tool(&call.name)?;
        let call_required = tool.permission_level_with_args(&call.arguments);
        if call_required > decision.allowed_permission {
            return Some(
                PolicyDenial::PermissionTooLow {
                    tool: &call.name,
                    required: call_required,
                    allowed: decision.allowed_permission,
                    channel: &self.channel,
                }
                .render(),
            );
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
            let content = match &decision {
                ToolPolicyDecision::RequireApproval { .. } => PolicyDenial::ApprovalRequired {
                    tool: &call.name,
                    policy: self.policy.name(),
                    reason,
                },
                _ => PolicyDenial::PolicyDenied {
                    tool: &call.name,
                    policy: self.policy.name(),
                    reason,
                },
            }
            .render();
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

/// `after_tool`: capture each tool call's execution outcome (success + content)
/// into a shared sink before the harness folds the result into a `Message::tool`
/// that drops the `error` flag (issue #4249). Without this, a post-turn
/// `ToolCallRecord` could only report every call as an optimistic success — the
/// in-house engine tracked real per-call success. The crate runs `after_tool` in
/// REVERSE registration order (issue #4464), so registering this AFTER the
/// summarization/cap middlewares (i.e. pushing it EARLIER, before
/// `TurnContextMiddleware::install`) makes its `after_tool` run AFTER those caps —
/// recording the final (summarized/capped) content the transcript keeps, not the
/// raw payload.
pub(crate) struct ToolOutcomeCaptureMiddleware {
    sink: super::ToolOutcomeSink,
    /// `call_id → (success, classified failure, elapsed, output chars)` fallback
    /// read by the event bridge when projecting `ToolCallCompleted`. TinyAgents
    /// 1.6 owns the raw outcome fields; the host still adds classified failure
    /// metadata for the UI.
    failure_map: super::observability::ToolFailureMap,
}

impl ToolOutcomeCaptureMiddleware {
    pub(crate) fn new(
        sink: super::ToolOutcomeSink,
        failure_map: super::observability::ToolFailureMap,
    ) -> Self {
        Self { sink, failure_map }
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
        // Enrich a raw security-policy / autonomy block (issue #4094): the ~20
        // `[policy-blocked]` denials emitted deep in `SecurityPolicy` / the tools
        // return a bare marker line with no workaround and no relay directive, so
        // the agent dead-ends. Rewrite the content into the structured
        // `Blocked / Reason / Workaround / relay` shape here — the last `after_tool`
        // hook, so the enriched text is what the transcript keeps. The marker is
        // preserved, and already-structured `ToolPolicyMiddleware` denials (which
        // carry a `Workaround:` suffix) are left untouched. This runs before
        // classification below, which still recognises the preserved marker.
        if let Some(enriched) =
            super::policy_denial::maybe_enrich_policy_block(&result.name, &result.content)
        {
            tracing::debug!(
                tool = result.name.as_str(),
                "[tinyagents::mw] enriched raw security-policy block with workaround + relay"
            );
            result.content = enriched;
        }

        let success = result.error.is_none();
        // Classify the failure so the live `ToolCallCompleted` event and the
        // persisted timeline can explain it in plain language. The classifier
        // owns all marker precedence now (policy-blocked / policy-denied / TTL
        // expiry short-circuit ahead of the `timed out` sniff — #4459), so this
        // just hands it the failure text.
        //
        // Sniff both `error` and `content`: the classifier historically read
        // `error` while the marker/timeout sniffs read `content`, a latent
        // asymmetry (#4459). Combine them so a marker/phrase is found wherever
        // the tool layer put it.
        let failure = if success {
            None
        } else {
            let error = result.error.as_deref().unwrap_or("");
            let combined: std::borrow::Cow<'_, str> = if error.is_empty() {
                std::borrow::Cow::Borrowed(result.content.as_str())
            } else if result.content.is_empty() || result.content == error {
                std::borrow::Cow::Borrowed(error)
            } else {
                std::borrow::Cow::Owned(format!("{error}\n{}", result.content))
            };
            let timed_out = combined.contains("timed out");
            Some(crate::openhuman::tool_status::classify(
                &combined, timed_out,
            ))
        };
        if let Ok(mut map) = self.failure_map.lock() {
            // Keep duration + rendered output size as a compatibility fallback
            // for old/deserialized completion events; TinyAgents 1.6 supplies
            // these fields directly on live `ToolCompleted` events.
            map.insert(
                result.call_id.clone(),
                (
                    success,
                    failure,
                    result.elapsed_ms,
                    result.content.chars().count(),
                ),
            );
        }
        if let Ok(mut sink) = self.sink.lock() {
            sink.push(super::ToolCallOutcome {
                call_id: result.call_id.clone(),
                name: result.name.clone(),
                success,
                content: result.content.clone(),
            });
        }
        Ok(())
    }
}

/// `before_tool`: repair a tool call's arguments *before* the harness runs its
/// fatal pre-execution schema gate (issues #4249 / #4451). A model can emit
/// arguments the model adapter parses to a non-object `Value` — invalid JSON
/// decodes to `Value::Null`, and some providers emit the whole arguments blob as
/// a JSON-encoded *string* (optionally wrapped in a ```json markdown fence). Left
/// alone the harness rejects those against an object schema and aborts the whole
/// turn.
///
/// Recovery, in order:
/// 1. Already a JSON object → leave it (the common, valid case).
/// 2. A JSON-encoded string (optionally fenced) that decodes to an object →
///    decode and use it.
/// 3. Otherwise a non-object whose tool schema declares **no** required fields →
///    coerce to `{}` (legacy-engine parity: the tool runs and produces its own
///    recoverable error).
/// 4. Otherwise (non-object + schema has required fields) → leave the arguments
///    untouched so [`SchemaGuardMiddleware`] converts the schema-validation
///    failure into a model-visible tool error rather than a turn abort. The old
///    behaviour (coerce to `{}`) *guaranteed* a `"<field> is required"` fatal
///    abort for those tools, so it is exactly the case that must fall through.
pub(crate) struct ArgRecoveryMiddleware {
    /// The same `Arc`-shared tool sets the runner registers, used to resolve a
    /// call's schema so we can tell whether coercing to `{}` is safe.
    tool_sets: Vec<Arc<Vec<Box<dyn Tool>>>>,
}

impl ArgRecoveryMiddleware {
    /// Build the middleware over the runner's shared tool sets.
    pub(crate) fn new(tool_sets: Vec<Arc<Vec<Box<dyn Tool>>>>) -> Self {
        Self { tool_sets }
    }

    fn schema_for(&self, name: &str) -> Option<ToolSchema> {
        schema_for_tool(&self.tool_sets, name)
    }
}

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
        // (1) Already valid object shape — nothing to do.
        if call.arguments.is_object() {
            return Ok(());
        }

        // (2) JSON-encoded-string arguments (optionally markdown-fenced): decode
        // and adopt the inner object.
        if let Some(raw) = call.arguments.as_str() {
            if let Some(obj) = recover_object_from_json_string(raw) {
                tracing::debug!(
                    tool = call.name.as_str(),
                    "[tinyagents::mw] arg_recovery: decoded JSON-encoded-string tool arguments to object"
                );
                call.arguments = obj;
                return Ok(());
            }
        }

        // (3) Non-object with a permissive schema (no required fields): coerce to
        // `{}` so the tool runs and produces its own recoverable error — engine
        // parity for tools that predate the schema gate.
        let has_required = self
            .schema_for(&call.name)
            .map(|schema| schema_has_required_fields(&schema.parameters))
            .unwrap_or(false);
        if !has_required {
            tracing::debug!(
                tool = call.name.as_str(),
                "[tinyagents::mw] arg_recovery: coercing non-object tool arguments to {{}} (schema declares no required fields)"
            );
            call.arguments = serde_json::json!({});
            return Ok(());
        }

        // (4) Non-object + schema has required fields: leave untouched. Coercing
        // to `{}` here would guarantee a fatal `"<field> is required"` abort;
        // instead `SchemaGuardMiddleware` surfaces a descriptive, recoverable
        // tool error.
        tracing::debug!(
            tool = call.name.as_str(),
            args_kind = json_value_kind(&call.arguments),
            "[tinyagents::mw] arg_recovery: leaving non-object tool arguments for the schema-guard tool-error path"
        );
        Ok(())
    }
}

/// `before_tool` + `wrap_tool`: convert the harness's **fatal** pre-execution
/// JSON-schema gate into a model-visible tool error instead of a turn abort
/// (issue #4451).
///
/// The tinyagents agent loop validates every tool call against its schema
/// (`ToolSchema::validate_call`) *between* `before_tool` and the tool-wrap onion;
/// any `required`/type/`enum` violation returns `TinyAgentsError::Validation`,
/// which propagates out of `run_loop` and fails the entire turn
/// (`"tinyagents harness run failed: Validation(...)"`). The legacy engine had no
/// such gate — bad arguments came back as recoverable tool *results* the model
/// self-corrected on the next iteration.
///
/// This middleware restores that behaviour entirely seam-side (the crate is
/// upstream/read-only):
/// - `before_tool` runs the *same* validation itself; on failure it records a
///   descriptive error keyed by the call id and rewrites the arguments to a
///   schema-satisfying **stub** so the crate's own fatal gate passes.
/// - `wrap_tool` then short-circuits the flagged call with a synthetic failed
///   [`TaToolResult`] **without** executing the real tool (the stub args never
///   reach it), so the loop continues and the model self-corrects.
///
/// Installed as the outermost tool-wrap middleware so an invalid call is turned
/// into a tool error before approval/policy wraps ever see the stub arguments.
pub(super) struct SchemaGuardMiddleware {
    /// The same `Arc`-shared tool sets the runner registers, used to resolve a
    /// call's schema for validation.
    tool_sets: Vec<Arc<Vec<Box<dyn Tool>>>>,
    /// call id → synthetic tool-error message, written in `before_tool` when a
    /// call fails validation and consumed in `wrap_tool` to short-circuit it.
    /// A flagged call always reaches `wrap_tool` (its stub args pass the crate
    /// gate), so entries never accumulate across a turn.
    pending: Arc<std::sync::Mutex<std::collections::HashMap<String, String>>>,
}

impl SchemaGuardMiddleware {
    /// Build the middleware over the runner's shared tool sets.
    pub(super) fn new(tool_sets: Vec<Arc<Vec<Box<dyn Tool>>>>) -> Self {
        Self {
            tool_sets,
            pending: Arc::default(),
        }
    }

    fn schema_for(&self, name: &str) -> Option<ToolSchema> {
        schema_for_tool(&self.tool_sets, name)
    }
}

#[async_trait]
impl Middleware<()> for SchemaGuardMiddleware {
    fn name(&self) -> &str {
        "schema_guard"
    }

    async fn before_tool(
        &self,
        _ctx: &mut RunContext<()>,
        _state: &(),
        call: &mut TaToolCall,
    ) -> TaResult<()> {
        // Unknown tool → let the crate's `UnknownToolPolicy` handle it (it
        // already returns a recoverable tool error).
        let Some(schema) = self.schema_for(&call.name) else {
            return Ok(());
        };

        let probe = TaToolCall {
            id: call.id.clone(),
            name: call.name.clone(),
            arguments: call.arguments.clone(),
        };
        let Err(err) = schema.validate_call(&probe) else {
            return Ok(());
        };

        let detail = match err {
            TinyAgentsError::Validation(message) => message,
            other => other.to_string(),
        };
        let schema_json = serde_json::to_string(&schema.parameters)
            .unwrap_or_else(|_| "<unavailable>".to_string());
        let message = format!(
            "invalid arguments for {}: {}. Expected schema: {}",
            call.name, detail, schema_json
        );
        tracing::warn!(
            tool = call.name.as_str(),
            detail = detail.as_str(),
            "[tinyagents::mw] schema_guard: tool-arg validation failed; converting fatal gate into a model-visible tool error"
        );

        if let Ok(mut pending) = self.pending.lock() {
            pending.insert(call.id.clone(), message);
        }
        // Rewrite to a schema-satisfying stub so the crate's fatal
        // `validate_call` gate passes; `wrap_tool` short-circuits the call
        // before these stub args can reach the real tool.
        call.arguments = synthesize_valid_arguments(&schema.parameters);
        Ok(())
    }
}

#[async_trait]
impl ToolMiddleware<()> for SchemaGuardMiddleware {
    fn name(&self) -> &str {
        "schema_guard"
    }

    async fn wrap_tool(
        &self,
        ctx: &mut RunContext<()>,
        state: &(),
        call: TaToolCall,
        next: ToolHandler<'_, (), ()>,
    ) -> TaResult<MiddlewareToolOutcome> {
        let flagged = self
            .pending
            .lock()
            .ok()
            .and_then(|mut pending| pending.remove(&call.id));
        if let Some(message) = flagged {
            tracing::debug!(
                tool = call.name.as_str(),
                "[tinyagents::mw] schema_guard: short-circuiting invalid tool call with a synthetic error result"
            );
            return Ok(MiddlewareToolOutcome::Result(TaToolResult::error(
                call.id, call.name, message,
            )));
        }
        next.run(ctx, state, call).await
    }
}

/// Resolves the harness [`ToolSchema`] for `name` across the runner's shared
/// tool sets.
///
/// Built via the same [`spec_to_schema`](super::convert::spec_to_schema)
/// conversion the runner uses for [`SharedToolAdapter::schema`], so the
/// `parameters` we validate against are byte-identical to the ones the crate's
/// fatal `validate_call` gate checks — otherwise our pre-validation could
/// disagree with the crate and either miss a fatal case or stub a call the crate
/// still rejects.
fn schema_for_tool(tool_sets: &[Arc<Vec<Box<dyn Tool>>>], name: &str) -> Option<ToolSchema> {
    tool_sets
        .iter()
        .flat_map(|set| set.iter())
        .find(|tool| tool.name() == name)
        .map(|tool| super::convert::spec_to_schema(&tool.spec()))
}

/// Whether a tool's JSON-schema `parameters` declares any `required` field.
fn schema_has_required_fields(parameters: &serde_json::Value) -> bool {
    parameters
        .get("required")
        .and_then(serde_json::Value::as_array)
        .map(|required| required.iter().any(serde_json::Value::is_string))
        .unwrap_or(false)
}

/// Attempts to recover a JSON **object** from a string-encoded arguments payload:
/// providers sometimes emit the whole arguments blob as a JSON string, optionally
/// wrapped in a ```json markdown fence. Returns `None` when the string does not
/// decode to a JSON object.
fn recover_object_from_json_string(raw: &str) -> Option<serde_json::Value> {
    let candidate = strip_code_fence(raw);
    serde_json::from_str::<serde_json::Value>(candidate)
        .ok()
        .filter(serde_json::Value::is_object)
}

/// Strips a surrounding markdown code fence (```` ```json … ``` ````) and its
/// optional language tag, returning the inner text. A string with no fence is
/// returned trimmed and unchanged.
fn strip_code_fence(raw: &str) -> &str {
    let trimmed = raw.trim();
    let Some(after_open) = trimmed.strip_prefix("```") else {
        return trimmed;
    };
    // Drop an optional language tag on the opening fence line (e.g. `json`).
    let body = match after_open.find('\n') {
        Some(newline)
            if after_open[..newline]
                .chars()
                .all(|c| c.is_ascii_alphanumeric()) =>
        {
            &after_open[newline + 1..]
        }
        _ => after_open,
    };
    body.trim().strip_suffix("```").unwrap_or(body).trim()
}

/// A short, human-readable kind label for a JSON value, for debug logging.
fn json_value_kind(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

/// Synthesizes a minimal value that satisfies the JSON-schema subset the
/// tinyagents gate (`ToolSchema::validate_call`) enforces — `enum`, `type`,
/// object `properties`/`required`, and array `items`. Used to rewrite a
/// validation-failed call's arguments so the crate's fatal gate passes; the call
/// is then short-circuited in `wrap_tool`, so this stub never reaches the tool.
fn synthesize_valid_arguments(schema: &serde_json::Value) -> serde_json::Value {
    use serde_json::Value;

    // `enum` constrains the value to a fixed set — the first option always
    // satisfies the gate (and any co-declared `type`).
    if let Some(values) = schema.get("enum").and_then(Value::as_array) {
        return values.first().cloned().unwrap_or(Value::Null);
    }

    // `type` may be a string or an array of strings; pick the first known kind.
    let kind = schema.get("type").and_then(|type_spec| {
        type_spec.as_str().map(str::to_string).or_else(|| {
            type_spec
                .as_array()?
                .iter()
                .filter_map(Value::as_str)
                .next()
                .map(str::to_string)
        })
    });

    match kind.as_deref() {
        Some("object") => synthesize_valid_object(schema),
        Some("array") => Value::Array(Vec::new()),
        Some("string") => Value::String(String::new()),
        Some("integer") | Some("number") => serde_json::json!(0),
        Some("boolean") => Value::Bool(false),
        Some("null") => Value::Null,
        _ => {
            if schema.get("properties").is_some() {
                synthesize_valid_object(schema)
            } else {
                // No understood constraints → an empty object trivially passes.
                Value::Object(serde_json::Map::new())
            }
        }
    }
}

/// Builds an object populated with every `required` field (recursively) so it
/// satisfies the gate's object/`required` checks.
fn synthesize_valid_object(schema: &serde_json::Value) -> serde_json::Value {
    use serde_json::Value;

    let mut object = serde_json::Map::new();
    if let Some(required) = schema.get("required").and_then(Value::as_array) {
        let properties = schema.get("properties").and_then(Value::as_object);
        for field in required.iter().filter_map(Value::as_str) {
            let field_schema = properties
                .and_then(|props| props.get(field))
                .cloned()
                .unwrap_or(Value::Null);
            object.insert(field.to_string(), synthesize_valid_arguments(&field_schema));
        }
    }
    Value::Object(object)
}

/// Agents are told to follow a **read-index → dedupe → write → update-index**
/// cycle around durable memory, but the contract was never enforced, so it was
/// followed inconsistently: writes landed without a dedupe read (duplicating
/// entries) and `update_memory_md` was skipped (so `MEMORY.md` drifted from the
/// store). This middleware observes the ordered sequence of *successful* memory
/// tool calls via [`MemoryProtocolTracker`] and, on each memory write, appends a
/// corrective note to the tool result so the model is nudged back onto the
/// protocol — the same "structured correction surfaced to the model" pattern the
/// unknown-tool recovery (#4118) uses. At run end it warns when a write was never
/// followed by an index update (the index is left stale).
///
/// Only *successful* ops advance the state machine — a failed `memory_store`
/// neither creates an entry nor obliges an index update. Non-memory tools are
/// ignored, so this is a no-op on turns that never touch memory.
pub struct MemoryProtocolMiddleware {
    tracker:
        std::sync::Mutex<crate::openhuman::agent::harness::memory_protocol::MemoryProtocolTracker>,
    /// call_id → classified op, captured in `before_tool` (the tool result carries
    /// no arguments, yet `update_memory_md` and `memory_tree` can only be
    /// classified from their `file` / `mode` argument). Correlated back by
    /// `result.call_id` in `after_tool`.
    pending_ops: std::sync::Mutex<
        std::collections::HashMap<
            String,
            crate::openhuman::agent::harness::memory_protocol::MemoryOp,
        >,
    >,
}

impl MemoryProtocolMiddleware {
    pub fn new() -> Self {
        Self {
            tracker: std::sync::Mutex::new(
                crate::openhuman::agent::harness::memory_protocol::MemoryProtocolTracker::new(),
            ),
            pending_ops: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }
}

impl Default for MemoryProtocolMiddleware {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Middleware<()> for MemoryProtocolMiddleware {
    fn name(&self) -> &str {
        "memory_protocol"
    }

    async fn before_tool(
        &self,
        _ctx: &mut RunContext<()>,
        _state: &(),
        call: &mut TaToolCall,
    ) -> TaResult<()> {
        // Classify with the arguments in hand (the result won't carry them) and
        // stash the op keyed by call id. Only memory-relevant ops are stored, so
        // the map stays empty on turns that never touch memory.
        let op = crate::openhuman::agent::harness::memory_protocol::classify_memory_op(
            &call.name,
            &call.arguments,
        );
        if op != crate::openhuman::agent::harness::memory_protocol::MemoryOp::Other {
            if let Ok(mut ops) = self.pending_ops.lock() {
                ops.insert(call.id.clone(), op);
            }
        }
        Ok(())
    }

    async fn after_tool(
        &self,
        _ctx: &mut RunContext<()>,
        _state: &(),
        result: &mut TaToolResult,
    ) -> TaResult<()> {
        // Consume the op captured for this call (removing it so the map can't
        // grow unbounded). Absent → a non-memory tool: nothing to enforce.
        let op = self
            .pending_ops
            .lock()
            .ok()
            .and_then(|mut ops| ops.remove(&result.call_id));
        let Some(op) = op else {
            return Ok(());
        };
        // Only successful memory ops advance the protocol — a failed write did
        // not mutate memory and must not demand an index update.
        if result.error.is_some() {
            return Ok(());
        }
        let observation = {
            let mut tracker = match self.tracker.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            tracker.observe(op)
        };
        if let Some(note) = observation.guidance(&result.name) {
            tracing::debug!(
                tool = result.name.as_str(),
                missing_index_read = observation.missing_index_read,
                index_drift = observation.index_drift,
                "[tinyagents::mw] memory-protocol guidance appended to tool result"
            );
            if !result.content.is_empty() {
                result.content.push_str("\n\n");
            }
            result.content.push_str(&note);
        }
        Ok(())
    }

    async fn after_agent(
        &self,
        _ctx: &mut RunContext<()>,
        _state: &(),
        _run: &mut AgentRun,
    ) -> TaResult<()> {
        let pending = self
            .tracker
            .lock()
            .map(|tracker| tracker.pending_index_update())
            .unwrap_or(false);
        if pending {
            tracing::warn!(
                "[tinyagents::mw] memory-protocol: run ended with a memory write that was never \
                 followed by update_memory_md — the MEMORY.md index is left stale"
            );
        }
        Ok(())
    }
}

/// `before_model`: enforce OpenHuman's daily/monthly cost budgets **before** a
/// model call spends (issue #4249, Phase 5). Reads the global
/// [`CostTracker`](crate::openhuman::cost) and, when cost budgets are configured
/// and already exceeded, fails the run before the provider call; a warning
/// threshold logs but proceeds. This enforcement path stays **authoritative**.
///
/// Self-gating: a no-op unless a global tracker exists and `config.enabled` with
/// a limit is set (`check_budget` returns `Allowed` otherwise). Complements the
/// post-call `StopHookMiddleware` per-turn USD cap. Projecting the *next* call's
/// cost pre-spend (vs the already-exceeded check here) needs an input-token
/// estimate — a follow-up.
///
/// # Shadow role (W2-budget-dedupe)
///
/// When built with [`with_shadow`](Self::with_shadow), this middleware is ALSO a
/// divergence-logging shadow over the observe-only crate
/// [`BudgetMiddleware`](tinyagents::harness::middleware::BudgetMiddleware). It
/// keeps enforcing exactly as before, but at `after_agent` it compares the
/// crate `BudgetMiddleware`'s shared [`BudgetTracker`] accumulation against the
/// authoritative runtime [`AgentRun::usage`] and logs `[budget_shadow]` parity
/// or divergence (compact numeric summary; no PII). Both accumulate the same
/// per-call `response.usage`, so token totals must match once the crate
/// middleware is on the path — this is the parity signal that must be clean
/// before enforcement can flip to the crate owner (see the flip-criteria comment
/// at the registration site in `tinyagents/mod.rs`). Cost is intentionally NOT
/// compared: the observe-only crate middleware has no pricing table, so its cost
/// stays zero while the local path prices via `cost::catalog` — cost parity is a
/// flip-criteria follow-up.
pub(crate) struct CostBudgetMiddleware {
    /// Observe-only crate `BudgetMiddleware`'s shared tracker handle, for the
    /// end-of-run `[budget_shadow]` comparison. `None` when the shadow is not
    /// installed (isolated unit tests of the enforcement gate).
    shadow_tracker: Option<BudgetTracker>,
}

impl CostBudgetMiddleware {
    /// Enforcement-only gate with no shadow comparison (isolated unit tests).
    pub(crate) fn new() -> Self {
        Self {
            shadow_tracker: None,
        }
    }

    /// Enforcement gate that ALSO compares its per-run token accounting against
    /// the observe-only crate `BudgetMiddleware`'s shared `tracker` at end of run
    /// and logs `[budget_shadow]` parity/divergence.
    pub(crate) fn with_shadow(tracker: BudgetTracker) -> Self {
        Self {
            shadow_tracker: Some(tracker),
        }
    }
}

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

    /// Shadow parity check (W2-budget-dedupe). Enforcement already happened per
    /// call in `before_model`; here we only observe. Compares the observe-only
    /// crate `BudgetMiddleware`'s accumulated token spend against the runtime's
    /// authoritative `AgentRun::usage` and logs `[budget_shadow]` divergence.
    /// Never fails the run.
    async fn after_agent(
        &self,
        _ctx: &mut RunContext<()>,
        _state: &(),
        run: &mut AgentRun,
    ) -> TaResult<()> {
        let Some(tracker) = &self.shadow_tracker else {
            return Ok(());
        };
        let crate_usage = tracker.snapshot().usage; // UsageTotals (crate shadow)
        let local = run.usage; // UsageTotals (runtime authoritative)
        let l = &local.usage;
        let c = &crate_usage.usage;
        let diverged = l.input_tokens != c.input_tokens
            || l.output_tokens != c.output_tokens
            || l.cache_read_tokens != c.cache_read_tokens
            || l.total_tokens != c.total_tokens
            || local.calls != crate_usage.calls;
        if diverged {
            tracing::warn!(
                local_calls = local.calls,
                crate_calls = crate_usage.calls,
                local_in = l.input_tokens,
                crate_in = c.input_tokens,
                local_out = l.output_tokens,
                crate_out = c.output_tokens,
                local_cached = l.cache_read_tokens,
                crate_cached = c.cache_read_tokens,
                local_total = l.total_tokens,
                crate_total = c.total_tokens,
                "[budget_shadow] divergence: crate BudgetMiddleware token accounting differs from authoritative AgentRun.usage"
            );
        } else {
            tracing::debug!(
                calls = local.calls,
                input = l.input_tokens,
                output = l.output_tokens,
                cached = l.cache_read_tokens,
                total = l.total_tokens,
                "[budget_shadow] parity: crate BudgetMiddleware token accounting matches AgentRun.usage"
            );
        }
        Ok(())
    }
}

/// `after_tool`: stop (or nudge) the run when tool calls keep failing with no
/// progress (issue #4249). The legacy tool loop's progress guard surfaced a
/// root-cause halt summary — a security/approval denial re-issued unchanged, an
/// identical error retried, or *different* commands all failing — instead of
/// burning the whole iteration budget and ending on a generic cap error. The
/// tinyagents path kept only the model/tool call caps, so this reinstates the
/// guard as a graph middleware.
///
/// As of tinyagents 1.5.0 the escalation ladder itself lives in the crate
/// ([`NoProgressTracker`], extracted upstream from OpenHuman #4389). This
/// middleware is now a **thin driver**: it captures the per-call argument
/// fingerprint (the tool result carries no arguments), feeds each outcome into
/// [`NoProgressTracker::record`], and lowers the returned [`NoProgress`] verdict
/// into OpenHuman steering. It owns only the OpenHuman-side policy:
///
/// - [`NoProgress::Continue`] — do nothing.
/// - [`NoProgress::Nudge`] — inject the crate's structured "no progress since
///   step X" corrective into the working transcript via
///   [`SteeringCommand::InjectMessage`] so the next model call sees it and
///   changes strategy *before* the same-strategy retry cap trips. (Not
///   `Redirect`: that verb is outside the Interactive steering allowlist and
///   would abort the turn — see the nudge call site.)
/// - [`NoProgress::Halt`] — record the crate's root-cause summary into the shared
///   [`HaltSummarySlot`](super::HaltSummarySlot) (the turn overrides its final
///   text with it) and pause the run via the shared steering handle (same
///   mechanism as the stop-hook / cap pausers), then [`reset`](NoProgressTracker::reset)
///   so a resumed run does not immediately re-pause on the latched state.
pub(crate) struct RepeatedToolFailureMiddleware {
    handle: SteeringHandle,
    halt_summary: super::HaltSummarySlot,
    /// Crate no-progress escalation ladder — the single source of the
    /// identical-failure / varied-failure / hard-reject logic (tinyagents 1.5.0).
    tracker: NoProgressTracker,
    /// Monotonic tool-outcome counter, used only for the crate's "no progress
    /// since step X" nudge wording. Not the model-call count, but a stable,
    /// increasing marker is all the wording needs.
    step: AtomicUsize,
    /// call_id → argument fingerprint, captured in `before_tool` (the tool result
    /// carries no arguments). Folded into the identical-repeat signature so the
    /// "identical arguments" halt only trips on the *same* args — two different
    /// argument sets that happen to share a first error line don't count as a
    /// repeat and can't pre-empt the generic no-progress backstop.
    arg_sigs: std::sync::Mutex<std::collections::HashMap<String, String>>,
    /// Recoverable-failure ladder (issue #4463): transient failures (timeouts,
    /// connection resets, rate limits, 5xx) are routed here instead of the crate
    /// tracker so they get the legacy extended headroom
    /// ([`RECOVERABLE_REPEAT_FAILURE_THRESHOLD`] identical /
    /// [`RECOVERABLE_NO_PROGRESS_FAILURE_THRESHOLD`] consecutive) rather than the
    /// crate's fixed 3/6, which is right only for deterministic failures.
    /// `tool\u{1f}args` → identical-failure count; persists across the turn.
    recoverable_sig_counts: std::sync::Mutex<std::collections::HashMap<String, u32>>,
    /// Consecutive recoverable-looking failures with no success in between. Reset
    /// on any success or non-recoverable failure (mirrors the legacy guard).
    recoverable_consecutive: AtomicU32,
}

impl RepeatedToolFailureMiddleware {
    /// Build the breaker. `identical_threshold` (the identical-signature retry
    /// ceiling) is handed straight to [`NoProgressTracker::new`], which clamps it
    /// so a nudge always precedes a halt (a single failure is never a loop).
    pub(crate) fn new(
        handle: SteeringHandle,
        identical_threshold: usize,
        halt_summary: super::HaltSummarySlot,
    ) -> Self {
        Self {
            handle,
            halt_summary,
            tracker: NoProgressTracker::new(identical_threshold),
            step: AtomicUsize::new(0),
            arg_sigs: std::sync::Mutex::new(std::collections::HashMap::new()),
            recoverable_sig_counts: std::sync::Mutex::new(std::collections::HashMap::new()),
            recoverable_consecutive: AtomicU32::new(0),
        }
    }

    /// Clear the consecutive recoverable-failure streak. Called on any success or
    /// non-recoverable failure (the per-signature identical counts persist across
    /// the turn, matching the legacy guard). Idempotent.
    fn reset_recoverable_streak(&self) {
        self.recoverable_consecutive.store(0, Ordering::SeqCst);
    }

    /// Record one recoverable failure and return a root-cause halt summary once
    /// its extended headroom is exhausted (identical `>=` [`RECOVERABLE_REPEAT_FAILURE_THRESHOLD`]
    /// or consecutive `>=` [`RECOVERABLE_NO_PROGRESS_FAILURE_THRESHOLD`]).
    fn record_recoverable(&self, tool: &str, arg_fp: &str, failure_text: &str) -> Option<String> {
        let key = format!("{tool}\u{1f}{arg_fp}");
        let count = self
            .recoverable_sig_counts
            .lock()
            .ok()
            .map(|mut counts| {
                let c = counts.entry(key).or_insert(0);
                *c += 1;
                *c
            })
            .unwrap_or(0);
        let consecutive = self.recoverable_consecutive.fetch_add(1, Ordering::SeqCst) + 1;
        tracing::debug!(
            tool,
            count,
            consecutive,
            "[tinyagents::mw] recoverable tool failure recorded with extended circuit-breaker headroom"
        );
        if count >= RECOVERABLE_REPEAT_FAILURE_THRESHOLD {
            return Some(recoverable_identical_halt_summary(
                tool,
                count,
                failure_text,
            ));
        }
        if consecutive >= RECOVERABLE_NO_PROGRESS_FAILURE_THRESHOLD {
            return Some(recoverable_no_progress_halt_summary(
                consecutive,
                tool,
                failure_text,
            ));
        }
        None
    }
}

/// Recognise a **user-actionable** blocker in a failing tool result — one only
/// the user can clear — and phrase the halt as a direct ask instead of the
/// crate's generic "the goal looks unreachable in this environment, report this
/// back" summary (issue #4092). Today that's a missing service connection (the
/// issue's canonical example: acting on a service that isn't connected). Such a
/// failure will never self-resolve by retrying, and the fix is the user's, so
/// escalate with a concrete next step instead of looping or reporting a generic
/// dead-end. Returns `None` for failures that are not user-actionable, leaving
/// the crate's summary in place.
fn user_actionable_escalation(tool: &str, error: &str) -> Option<String> {
    let lower = error.to_lowercase();
    let permission_or_scope_failure = lower.contains("[composio:error:insufficient_scope]")
        || lower.contains("[composio:error:trigger_permission]")
        || lower.contains("insufficient scope")
        || lower.contains("insufficient authentication scopes")
        || lower.contains("insufficient permissions")
        || lower.contains("missing required permissions")
        || lower.contains("permission to manage triggers");
    if permission_or_scope_failure {
        return None;
    }
    // Keep this narrow: some scope/permission failures legitimately tell the
    // user to reconnect in Settings, but they are not missing connections.
    let missing_connection = lower.contains("[composio:error:composio_platform]")
        || lower.contains("not connected")
        || lower.contains("isn't connected")
        || lower.contains("is not connected")
        || lower.contains("not enabled")
        || lower.contains("token revoked")
        || lower.contains("connection error, try to authenticate");
    if !missing_connection {
        return None;
    }
    Some(format!(
        "I can't continue without your input: the `{tool}` action needs a service that isn't \
         connected. {}\n\nConnect it (Settings \u{2192} Connections), then tell me to retry — or \
         tell me how you'd like to proceed instead.",
        crate::openhuman::util::truncate_with_ellipsis(error, 400),
    ))
}

/// A stable, bounded fingerprint of a tool call's arguments for the identical-
/// repeat signature (hashed so a huge payload doesn't bloat the map/comparison).
fn args_fingerprint(arguments: &serde_json::Value) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    arguments.to_string().hash(&mut hasher);
    format!("{:x}", hasher.finish())
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
        let arg_fp = self
            .arg_sigs
            .lock()
            .ok()
            .and_then(|mut sigs| sigs.remove(&result.call_id))
            .unwrap_or_default();
        let step = self.step.fetch_add(1, Ordering::SeqCst) + 1;

        // Combined failure text for classification: the model-facing content plus
        // the (redundant but authoritative) error field. Both are scanned for the
        // policy / terminal-inference / recoverable markers below.
        let failure_text = match result.error.as_deref() {
            Some(err) => format!("{}\n{}", result.content, err),
            None => String::new(),
        };

        // ── Part 5 (#3104): terminal delegated-inference fast-halt ──────────────
        // A permanent inference failure (out of budget / provider-config rejection)
        // surfaced by a delegated sub-agent cannot be recovered by retrying — the
        // budget is account-wide and the model/provider config is shared by every
        // (sub-)agent. Halt on the FIRST occurrence with an actionable root cause,
        // *before* the count-based thresholds, because the orchestrator otherwise
        // re-emits the doomed step under varied delegation-tool names so the
        // identical-retry threshold never trips in time.
        if result.error.is_some() {
            if let Some(kind) = terminal_inference_failure_kind(&failure_text) {
                tracing::warn!(
                    tool = %result.name,
                    kind = ?kind,
                    "[tinyagents::mw] terminal delegated-inference failure — halting on first occurrence with root cause"
                );
                if let Ok(mut slot) = self.halt_summary.lock() {
                    *slot = Some(terminal_inference_halt_summary(
                        kind,
                        &result.name,
                        &failure_text,
                    ));
                }
                self.handle.send(SteeringCommand::Pause);
                self.tracker.reset();
                self.reset_recoverable_streak();
                return Ok(());
            }
        }

        // A hard policy rejection is marked in the tool output; it can never
        // succeed when re-issued unchanged, so the crate ladder trips it faster
        // (its `HARD_REJECT_HALT_THRESHOLD` of 2). Both the read-only/forbidden
        // block (`POLICY_BLOCKED_MARKER`) and the approval denial / TTL expiry
        // (`POLICY_DENIED_MARKER`) are deterministic — restore the 2-repeat
        // fast-trip for BOTH (issue #4463 part 6: denied had drifted to the
        // generic 3).
        let policy_marked = |s: &str| {
            s.contains(crate::openhuman::security::POLICY_BLOCKED_MARKER)
                || s.contains(crate::openhuman::security::POLICY_DENIED_MARKER)
        };
        let hard_reject =
            policy_marked(&result.content) || result.error.as_deref().is_some_and(policy_marked);

        // ── Part 4: recoverable-failure headroom ────────────────────────────────
        // Transient failures (timeouts, connection resets, rate limits, 5xx) get
        // the legacy extended headroom instead of the crate's deterministic 3/6.
        // Route them to the recoverable ladder; a success or a non-recoverable
        // failure resets that streak and feeds the crate tracker as before.
        let recoverable = result.error.is_some()
            && !hard_reject
            && (is_recoverable_tool_failure(&failure_text)
                || matches!(
                    crate::openhuman::tool_status::classify(&failure_text, false).class,
                    crate::openhuman::tool_status::ToolFailureClass::Timeout
                        | crate::openhuman::tool_status::ToolFailureClass::ServiceUnavailable
                        | crate::openhuman::tool_status::ToolFailureClass::ModelConnection
                ));
        if recoverable {
            if let Some(summary) = self.record_recoverable(&result.name, &arg_fp, &failure_text) {
                tracing::warn!(
                    tool = %result.name,
                    "[tinyagents::mw] recoverable-failure headroom exhausted — halting run so the root cause surfaces"
                );
                if let Ok(mut slot) = self.halt_summary.lock() {
                    *slot = Some(summary);
                }
                self.handle.send(SteeringCommand::Pause);
                self.reset_recoverable_streak();
            }
            // Recoverable failures never feed the crate tracker — its fixed 3/6
            // backstop would halt them before the extended headroom is spent.
            return Ok(());
        }
        // Success or non-recoverable failure: clear the recoverable streak (its
        // per-signature counts persist across the turn) before the crate tracker
        // handles the deterministic 3/6 + hard-reject-2 path below.
        self.reset_recoverable_streak();

        let attempt = ToolAttempt {
            tool: &result.name,
            arg_fingerprint: &arg_fp,
            error: result.error.as_deref(),
            hard_reject,
            // The unknown-tool recovery sentinel is a C3 concern; today every
            // failure feeds the generic backstop exactly as the legacy ladder did.
            recoverable_miss: false,
        };

        match self.tracker.record(step, &attempt) {
            NoProgress::Continue => {}
            NoProgress::Nudge(instruction) => {
                tracing::warn!(
                    tool = %result.name,
                    step,
                    hard_reject,
                    "[tinyagents::mw] no-progress nudge — steering the model to change strategy before the retry cap"
                );
                // Inject the crate's structured corrective as a system message via
                // the `InjectMessage` steering lane. This runs on *every* turn,
                // including the user's live interactive turn, whose steering policy
                // permits only `InjectMessage`/`Pause` — `Redirect` is Background
                // (sub-agent) only, so sending it here aborted every interactive
                // turn that hit the nudge with `steering command redirect is not
                // permitted by the run policy` (a #4473 migration regression). The
                // corrective is trusted, system-generated advisory text, so the
                // `InjectMessage` lane is both permitted and semantically correct.
                self.handle
                    .send(SteeringCommand::InjectMessage(TaMessage::system(
                        instruction,
                    )));
            }
            NoProgress::Halt(summary) => {
                // #4092: if the blocker is user-actionable (a missing connection),
                // escalate with a concrete ask instead of the crate's generic
                // "unreachable environment, report back" summary.
                let escalation = user_actionable_escalation(
                    &result.name,
                    result.error.as_deref().unwrap_or(result.content.as_str()),
                );
                let user_actionable = escalation.is_some();
                let summary = escalation.unwrap_or(summary);
                tracing::warn!(
                    tool = %result.name,
                    step,
                    hard_reject,
                    user_actionable,
                    "[tinyagents::mw] repeated tool failure — halting run so the root cause surfaces"
                );
                if let Ok(mut slot) = self.halt_summary.lock() {
                    *slot = Some(summary);
                }
                // Pause at the top of the next iteration (before the next model
                // call), matching the stop-hook / cap pause path. Reset so a
                // resumed run does not immediately re-pause on the latched state
                // (the crate also resets internally on a halt; this is explicit
                // and idempotent).
                self.handle.send(SteeringCommand::Pause);
                self.tracker.reset();
            }
        }
        Ok(())
    }
}

// ── Loop-guard restorations (issue #4463) ────────────────────────────────────
//
// The TinyAgents migration dropped several loop breakers that the crate does not
// replace (verified against `harness::no_progress`, which tracks *failures*
// only): the recoverable-failure headroom, the terminal delegated-inference
// fast-halt (#3104), the policy-denied fast-trip, and the successful-repeat /
// identical-output guards (#4088 / #4095). These helpers + the
// [`RepeatProgressMiddleware`] below restore that behaviour seam-side, ported
// verbatim from the deleted `agent/harness/tool_loop.rs` thresholds/wording so
// the guards read identically to the legacy loop.

/// Recoverable/transient failures get more identical-retry headroom than the
/// deterministic default: a flaky network call or a timeout can succeed on a
/// later attempt once the model adapts (longer timeout, smaller batch, retry).
/// Mirrors the legacy `RECOVERABLE_REPEAT_FAILURE_THRESHOLD`.
const RECOVERABLE_REPEAT_FAILURE_THRESHOLD: u32 = 8;
/// Recoverable failures also get a larger *consecutive* (varied-args) no-progress
/// headroom before the breaker halts. Mirrors the legacy
/// `RECOVERABLE_NO_PROGRESS_FAILURE_THRESHOLD`.
const RECOVERABLE_NO_PROGRESS_FAILURE_THRESHOLD: u32 = 12;

/// The model re-emitting the IDENTICAL assistant output (narration + the same
/// tool call) this many times in a row is a no-progress narration loop — halt.
/// Mirrors the legacy `REPEAT_OUTPUT_THRESHOLD` (#4095).
const REPEAT_OUTPUT_THRESHOLD: u32 = 4;

/// The model re-issuing the IDENTICAL `(tool, args)` batch this many times in a
/// row — regardless of whether each call *succeeds* — is spinning one action
/// with no new information. Set just below [`REPEAT_OUTPUT_THRESHOLD`] so a
/// verbatim call loop is caught a step earlier than the broader narration loop.
/// Mirrors the legacy `REPEAT_CALL_THRESHOLD` (#4088).
const REPEAT_CALL_THRESHOLD: u32 = 3;

/// Clamp the last-error text embedded in a circuit-breaker halt summary so a huge
/// tool error (already capped at 1MB upstream) can't blow up the agent's result.
/// Mirrors the legacy `tool_loop::truncate_for_halt`.
fn truncate_for_halt(s: &str) -> String {
    const MAX: usize = 600;
    if s.chars().count() <= MAX {
        return s.to_string();
    }
    let head: String = s.chars().take(MAX).collect();
    format!("{head}\n… [truncated]")
}

/// Failures that are informative and plausibly recoverable by changing the next
/// action (longer timeout, smaller batch, different network retry/fallback)
/// rather than by abandoning the turn. Deliberately marker-based and
/// conservative: it only controls breaker headroom, never converts a failure
/// into success. Ported verbatim from legacy `tool_loop::is_recoverable_tool_failure`.
fn is_recoverable_tool_failure(result: &str) -> bool {
    let lower = result.to_ascii_lowercase();
    [
        "timed out",
        "timeout",
        "deadline exceeded",
        "temporarily unavailable",
        "temporary failure",
        "connection reset",
        "connection refused",
        "connection closed",
        "connection aborted",
        "network is unreachable",
        "host is unreachable",
        "dns error",
        "failed to lookup address",
        "failed to resolve",
        "rate limit",
        "too many requests",
        "retry after",
        "503 service unavailable",
        "502 bad gateway",
        "504 gateway timeout",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

/// A permanent, non-retryable inference failure surfaced by a delegated
/// sub-agent's tool result. Unlike a transient error, re-issuing the call cannot
/// succeed even under a *different* delegation tool or varied args: the budget is
/// account-wide and the model/provider configuration is shared by every
/// (sub-)agent. See [`terminal_inference_failure_kind`] (#3104).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum TerminalInferenceFailure {
    /// Out of inference budget / credits — every retry hits the same wall.
    BudgetExhausted,
    /// The configured model/provider rejected the request for a reason the user
    /// must fix (unknown model, non-chat/embedding model, missing credential,
    /// region block, …).
    ProviderConfig,
}

/// Inference/delegation **envelope** markers that prove a tool result came from a
/// delegated inference call (a sub-agent / provider round-trip) rather than from
/// arbitrary tool stderr. Every marker here is harness-generated (our own
/// reliable-chain rollup or sub-agent dispatch wrapper), NOT a provider HTTP body
/// that arbitrary tool stderr could forge. Ported from legacy `tool_loop`.
const INFERENCE_FAILURE_ENVELOPE_MARKERS: &[&str] = &[
    // Reliable-chain exhaustion rollup (reliable.rs::format_failure_aggregate).
    "all providers/models failed",
    "may not be available on your provider",
    // Sub-agent delegation failure wrapper (dispatch.rs::format_subagent_failure).
    "failed and did not complete",
];

/// True if `result` carries one of the inference/delegation envelope markers —
/// i.e. the failure demonstrably came from a delegated provider round-trip, not
/// arbitrary tool stderr. See [`INFERENCE_FAILURE_ENVELOPE_MARKERS`].
fn has_inference_failure_envelope(result: &str) -> bool {
    let lower = result.to_ascii_lowercase();
    INFERENCE_FAILURE_ENVELOPE_MARKERS
        .iter()
        .any(|marker| lower.contains(marker))
}

/// Recognize a permanent (non-retryable) delegated-inference failure from a tool
/// result. Two-stage gate so a *recoverable* tool failure can't be misclassified:
/// (1) the result must carry a delegated-inference envelope
/// ([`has_inference_failure_envelope`]); (2) the trusted body is matched against
/// the two tight provider classifiers. Budget takes precedence if both match.
/// Ported from legacy `tool_loop::terminal_inference_failure_kind` (#3104).
pub(crate) fn terminal_inference_failure_kind(result: &str) -> Option<TerminalInferenceFailure> {
    use crate::openhuman::inference::provider::{
        is_budget_exhausted_message, is_provider_config_rejection_message,
    };
    if !has_inference_failure_envelope(result) {
        return None;
    }
    if is_budget_exhausted_message(result) {
        Some(TerminalInferenceFailure::BudgetExhausted)
    } else if is_provider_config_rejection_message(result) {
        Some(TerminalInferenceFailure::ProviderConfig)
    } else {
        None
    }
}

/// The actionable root-cause halt summary for a terminal delegated-inference
/// failure. Ported verbatim from the legacy loop.
fn terminal_inference_halt_summary(
    kind: TerminalInferenceFailure,
    tool: &str,
    result: &str,
) -> String {
    match kind {
        TerminalInferenceFailure::BudgetExhausted => format!(
            "Stopping: the `{tool}` step failed because the account is out of inference \
             budget/credits — every retry hits the same wall. Add credits to your account \
             (or, when using a custom/BYO provider, top up that provider's own account) and try \
             again. Details:\n{}",
            truncate_for_halt(result),
        ),
        TerminalInferenceFailure::ProviderConfig => format!(
            "Stopping: the `{tool}` step failed because the configured model/provider rejected the \
             request (e.g. an unknown model, a non-chat/embedding model, a missing credential, or \
             a region block) — retrying will not help. Fix the model or API key in Settings → AI. \
             Details:\n{}",
            truncate_for_halt(result),
        ),
    }
}

/// Halt summary when a single recoverable `(tool, args)` call exhausts its
/// extended identical-retry headroom. Ported from the legacy loop.
fn recoverable_identical_halt_summary(tool: &str, count: u32, result: &str) -> String {
    format!(
        "Stopping: the `{tool}` call was retried {count} times with identical arguments and kept \
         failing — repeating it will not help. Last error:\n{}\n\nThis looked recoverable at \
         first, but the same call exhausted the extended transient-failure headroom. Report this \
         back instead of retrying.",
        truncate_for_halt(result),
    )
}

/// Halt summary when many recoverable-looking failures pile up with no progress.
/// Ported from the legacy loop.
fn recoverable_no_progress_halt_summary(consecutive: u32, tool: &str, result: &str) -> String {
    format!(
        "Stopping: {consecutive} recoverable-looking tool failures happened in a row with no \
         successful progress. Last error (from `{tool}`):\n{}\n\nThe turn is still bounded by the \
         iteration/cost limits, but this many consecutive transient failures means the goal is not \
         currently reachable. Report this back instead of retrying.",
        truncate_for_halt(result),
    )
}

/// Tools whose contract is to be re-invoked with identical arguments, so an
/// identical repeat is legitimate progress — not a no-progress loop. Today this
/// is `wait_subagent`, which polls a running async sub-agent and explicitly tells
/// the model to "call wait_subagent again" when a `timeout_secs` window elapses
/// while the sub-agent is still running. Without this exemption a task that
/// outlives two wait windows would have its third identical `wait_subagent`
/// halted by the no-progress breakers before it could collect the eventual
/// result. Ported from legacy `tool_loop::is_repeat_call_exempt` (Codex P1 on #4230).
pub(crate) fn is_repeat_call_exempt(tool: &str) -> bool {
    matches!(tool, "wait_subagent")
}

/// Extract the assistant's visible text (concatenated [`ContentBlock::Text`]
/// blocks) from a model response message, for the repeat-output signature.
fn assistant_visible_text(message: &tinyagents::harness::message::AssistantMessage) -> String {
    let mut out = String::new();
    for block in &message.content {
        if let ContentBlock::Text(t) = block {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(t);
        }
    }
    out
}

/// A back-to-back identical-signature streak counter. Trips (`record` returns the
/// new consecutive count) once the same hashed signature repeats; a different
/// signature resets the run. Backs both the repeat-output and repeat-call guards.
#[derive(Default)]
struct StreakGuard {
    last_hash: Option<u64>,
    consecutive: u32,
}

impl StreakGuard {
    /// Record one signature; returns the new consecutive count for that signature
    /// (1 after a reset). A different signature resets the streak to 1.
    fn record(&mut self, signature: &str) -> u32 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        signature.hash(&mut hasher);
        let h = hasher.finish();
        if self.last_hash == Some(h) {
            self.consecutive += 1;
        } else {
            self.last_hash = Some(h);
            self.consecutive = 1;
        }
        self.consecutive
    }

    /// Clear the streak — used when an iteration is a legitimately-repeating
    /// poll/wait (see [`is_repeat_call_exempt`]) or a failing batch that another
    /// guard owns, so it counts as a distinct action rather than a repeat.
    fn reset(&mut self) {
        self.last_hash = None;
        self.consecutive = 0;
    }
}

/// Per-batch state the repeat-CALL guard needs but can only fully evaluate once
/// every tool result in the assistant's batch has come back: the canonical
/// `(tool, args)` signature captured at `after_model`, plus the running
/// success/remaining accounting folded in at each `after_tool`.
#[derive(Default)]
struct PendingCallBatch {
    /// Canonical `(tool, args)` signature of the batch, from `after_model`.
    call_sig: String,
    /// Tool results still outstanding for this batch.
    remaining: usize,
    /// `true` while every result so far in the batch has succeeded.
    all_ok: bool,
    /// `true` when every call in the batch is a polling/wait exemption.
    exempt: bool,
}

/// Restores the deleted successful-repeat / identical-output loop breakers
/// (#4088 / #4095) as a seam middleware. The crate `no_progress` ladder (driving
/// [`RepeatedToolFailureMiddleware`]) resets on every success, so a model looping
/// on a *successful* no-op tool or re-emitting an identical narration+call never
/// trips it and burns the whole iteration budget. This guard closes both gaps:
///
/// - **Repeat-output** (`after_model`, checked before the tools run): halts when
///   the assistant's visible text + tool-call `(name, args)` batch is byte
///   identical [`REPEAT_OUTPUT_THRESHOLD`] iterations in a row.
/// - **Repeat-call** (evaluated once the batch's tool results are all back, gated
///   on every call succeeding): halts when the `(tool, args)` batch alone repeats
///   [`REPEAT_CALL_THRESHOLD`] times — catching successful no-op loops that vary
///   only their narration.
///
/// Polling/wait tools ([`is_repeat_call_exempt`]) are exempt from both: their
/// contract is to be re-invoked identically, so an all-poll batch resets the
/// streaks instead of recording. On a trip it writes the legacy root-cause
/// summary into the shared [`HaltSummarySlot`](super::HaltSummarySlot) and pauses
/// the run through the shared steering handle — the same halt mechanism as the
/// repeated-failure breaker.
pub(crate) struct RepeatProgressMiddleware {
    handle: SteeringHandle,
    halt_summary: super::HaltSummarySlot,
    /// Narration+call identical-output streak (#4095), threshold
    /// [`REPEAT_OUTPUT_THRESHOLD`].
    output_guard: std::sync::Mutex<StreakGuard>,
    /// `(tool, args)`-only successful-batch streak (#4088), threshold
    /// [`REPEAT_CALL_THRESHOLD`].
    call_guard: std::sync::Mutex<StreakGuard>,
    /// Batch bookkeeping bridging `after_model` → `after_tool` for the call guard.
    pending: std::sync::Mutex<Option<PendingCallBatch>>,
}

impl RepeatProgressMiddleware {
    pub(crate) fn new(handle: SteeringHandle, halt_summary: super::HaltSummarySlot) -> Self {
        Self {
            handle,
            halt_summary,
            output_guard: std::sync::Mutex::new(StreakGuard::default()),
            call_guard: std::sync::Mutex::new(StreakGuard::default()),
            pending: std::sync::Mutex::new(None),
        }
    }

    /// Latch a root-cause halt: record the summary the turn surfaces instead of an
    /// empty/last-model reply, and pause at the top of the next iteration (before
    /// the next model call), matching the repeated-failure breaker's halt path.
    fn halt(&self, summary: String) {
        if let Ok(mut slot) = self.halt_summary.lock() {
            *slot = Some(summary);
        }
        self.handle.send(SteeringCommand::Pause);
    }
}

#[async_trait]
impl Middleware<()> for RepeatProgressMiddleware {
    fn name(&self) -> &str {
        "repeat_progress"
    }

    async fn after_model(
        &self,
        _ctx: &mut RunContext<()>,
        _state: &(),
        response: &mut ModelResponse,
    ) -> TaResult<()> {
        let tool_calls = &response.message.tool_calls;
        if tool_calls.is_empty() {
            // A final answer (no tool calls) ends the loop; nothing to guard, and
            // there is no batch to track for the call guard.
            if let Ok(mut pending) = self.pending.lock() {
                *pending = None;
            }
            return Ok(());
        }

        // Polling/wait tools are contractually re-invoked with identical args +
        // narration each timeout while the work is still running, so an all-poll
        // batch is legitimate progress, not a no-progress repeat.
        let all_exempt = tool_calls.iter().all(|c| is_repeat_call_exempt(&c.name));

        // Canonical `(tool, args)` batch signature (call guard) and the broader
        // narration+call signature (output guard). Both fold each call in order
        // with a `\u{1}` separator, matching the legacy signatures.
        let mut call_sig = String::new();
        for call in tool_calls {
            call_sig.push('\u{1}');
            call_sig.push_str(&call.name);
            call_sig.push('\u{1}');
            call_sig.push_str(&call.arguments.to_string());
        }
        let output_sig = format!(
            "{}{}",
            assistant_visible_text(&response.message).trim(),
            call_sig
        );

        // Repeat-OUTPUT guard, checked BEFORE the (repeated) tools run so we don't
        // burn another no-op iteration.
        if all_exempt {
            if let Ok(mut g) = self.output_guard.lock() {
                g.reset();
            }
        } else {
            let consecutive = self
                .output_guard
                .lock()
                .map(|mut g| g.record(&output_sig))
                .unwrap_or(0);
            if consecutive >= REPEAT_OUTPUT_THRESHOLD {
                tracing::warn!(
                    consecutive,
                    "[tinyagents::mw] repeat-output circuit breaker tripped — identical response+tool-call repeated; halting"
                );
                self.halt(format!(
                    "Stopping: the last {consecutive} iterations produced the IDENTICAL response \
                     and tool call with no change — the run is stuck repeating the same step \
                     without making progress. Re-issuing it will not help. Summarise what (if \
                     anything) was actually accomplished and report that the task could not \
                     progress, or take a genuinely different approach.",
                ));
            }
        }

        // Stage the batch for the repeat-CALL guard, evaluated once every result
        // is back (gated on success) in `after_tool`.
        if let Ok(mut pending) = self.pending.lock() {
            *pending = Some(PendingCallBatch {
                call_sig,
                remaining: tool_calls.len(),
                all_ok: true,
                exempt: all_exempt,
            });
        }
        Ok(())
    }

    async fn after_tool(
        &self,
        _ctx: &mut RunContext<()>,
        _state: &(),
        result: &mut TaToolResult,
    ) -> TaResult<()> {
        // Fold this result into the pending batch; only act once the batch is
        // complete so the call guard sees whole-batch success.
        let completed = {
            let Ok(mut pending) = self.pending.lock() else {
                return Ok(());
            };
            let Some(batch) = pending.as_mut() else {
                return Ok(());
            };
            if result.error.is_some() {
                batch.all_ok = false;
            }
            batch.remaining = batch.remaining.saturating_sub(1);
            if batch.remaining == 0 {
                pending.take()
            } else {
                None
            }
        };
        let Some(batch) = completed else {
            return Ok(());
        };

        // Repeat-CALL breaker for SUCCESSFUL no-op loops (#4088): the failure
        // breaker owns repeated *failures* and resets on success, so an identical
        // call that keeps SUCCEEDING slips past it. A failing batch (its domain)
        // or an all-poll exemption resets the streak instead of recording.
        if batch.exempt || !batch.all_ok {
            if let Ok(mut g) = self.call_guard.lock() {
                g.reset();
            }
            return Ok(());
        }
        let consecutive = self
            .call_guard
            .lock()
            .map(|mut g| g.record(&batch.call_sig))
            .unwrap_or(0);
        if consecutive >= REPEAT_CALL_THRESHOLD {
            tracing::warn!(
                consecutive,
                "[tinyagents::mw] repeat-call circuit breaker tripped — identical successful (tool,args) batch repeated; halting"
            );
            self.halt(format!(
                "Stopping: the same tool call was issued {consecutive} times in a row with \
                 identical arguments and no new information — the run is stuck repeating one \
                 action without making progress. Re-issuing it will not help. Summarise what (if \
                 anything) was actually accomplished and report that the task could not progress, \
                 or take a genuinely different action (a different tool, different arguments, or \
                 hand back).",
            ));
        }
        Ok(())
    }
}

// ── ImageAwareMessageTrimMiddleware ───────────────────────────────────────────

/// Flat token cost charged per image — an inline `[IMAGE:…]` marker or a native
/// [`ContentBlock::Image`] block — instead of counting the base64 payload as
/// text. Restores the legacy `harness/token_budget.rs` semantics (issue #4462):
/// the crate `estimate_tokens` prices text at chars/4, so a single large base64
/// image reads as ~2M tokens and the trim believes the context is massively over
/// budget, evicting the whole transcript (system messages included). Providers
/// bill an image at ≈85–1100 tokens by detail; 1200 is a conservative upper
/// bound that keeps the budget realistic without the base64 payload inflating it.
const IMAGE_MARKER_TOKEN_COST: u64 = 1_200;

/// Inline image-marker prefix produced by the multimodal composer
/// (`agent/multimodal.rs`, `compose_multimodal_message`). Priced at
/// [`IMAGE_MARKER_TOKEN_COST`] rather than by its base64 length.
const IMAGE_MARKER_PREFIX: &str = "[IMAGE:";

/// Minimum reply/output reserve — mirrors the legacy `MIN_OUTPUT_RESERVE_TOKENS`.
const MIN_OUTPUT_RESERVE_TOKENS: u64 = 512;

/// Upper anchor for the reply/output reserve — mirrors the legacy
/// `DEFAULT_OUTPUT_RESERVE_TOKENS`.
const DEFAULT_OUTPUT_RESERVE_TOKENS: u64 = 8_192;

/// Rough token estimate (~4 characters per token) with inline `[IMAGE:…]`
/// markers charged a flat [`IMAGE_MARKER_TOKEN_COST`] instead of their base64
/// length. Mirrors the deleted `token_budget::estimate_tokens` (issue #4462).
/// Markerless text takes the fast char/4 path.
fn estimate_text_tokens(text: &str) -> u64 {
    if !text.contains(IMAGE_MARKER_PREFIX) {
        return (text.len() as u64).saturating_add(3) / 4;
    }
    let mut text_bytes: u64 = 0;
    let mut images: u64 = 0;
    let mut cursor = 0usize;
    while let Some(rel) = text[cursor..].find(IMAGE_MARKER_PREFIX) {
        let start = cursor + rel;
        text_bytes = text_bytes.saturating_add((start - cursor) as u64); // preceding text
        let after = start + IMAGE_MARKER_PREFIX.len();
        match text[after..].find(']') {
            Some(rel_end) => {
                images += 1;
                cursor = after + rel_end + 1; // skip the whole marker payload
            }
            None => {
                // Unterminated marker — count the remainder as text and stop.
                text_bytes = text_bytes.saturating_add((text.len() - start) as u64);
                cursor = text.len();
                break;
            }
        }
    }
    text_bytes = text_bytes.saturating_add((text.len() - cursor) as u64); // trailing text
    (text_bytes.saturating_add(3) / 4)
        .saturating_add(images.saturating_mul(IMAGE_MARKER_TOKEN_COST))
}

/// Count native [`ContentBlock::Image`] blocks on a message. `Message::text()`
/// concatenates only text blocks, so a native multimodal image would otherwise
/// contribute zero tokens; we charge each one [`IMAGE_MARKER_TOKEN_COST`].
fn count_native_image_blocks(msg: &TaMessage) -> u64 {
    let content = match msg {
        TaMessage::System(m) => &m.content,
        TaMessage::User(m) => &m.content,
        TaMessage::Assistant(m) => &m.content,
        TaMessage::Tool(m) => &m.content,
    };
    content
        .iter()
        .filter(|b| matches!(b, ContentBlock::Image(_)))
        .count() as u64
}

/// Estimate the tokens of a crate [`TaMessage`]: image-aware text tokens, a flat
/// [`IMAGE_MARKER_TOKEN_COST`] per native image block, and the assistant's
/// tool-call name/arguments (which `Message::text()` drops). Mirrors the legacy
/// `estimate_conversation_message_tokens` (issue #4462).
fn estimate_message_tokens(msg: &TaMessage) -> u64 {
    let mut total = estimate_text_tokens(&msg.text());
    total = total
        .saturating_add(count_native_image_blocks(msg).saturating_mul(IMAGE_MARKER_TOKEN_COST));
    if let TaMessage::Assistant(m) = msg {
        for call in &m.tool_calls {
            total = total.saturating_add(estimate_text_tokens(&call.name));
            total = total.saturating_add(estimate_text_tokens(&call.arguments.to_string()));
        }
    }
    total
}

/// Reply/output reserve, mirroring the legacy proportional clamp
/// `clamp(window/10, ≥512, ≤max(8192, window/4))`. Restores the small-window
/// budget the fixed `window − AGENT_TURN_MAX_OUTPUT_TOKENS` regressed (issue
/// #4462): an 8k model reserves ~819 tokens (input budget ~7373), not
/// 16384 → floored 1024.
fn legacy_output_reserve_tokens(window: u64) -> u64 {
    let pct = window / 10;
    pct.max(MIN_OUTPUT_RESERVE_TOKENS)
        .min(DEFAULT_OUTPUT_RESERVE_TOKENS.max(window / 4))
}

/// Input-prompt token budget after reserving room for the reply. Public to the
/// seam so the install site (and tests) can assert the legacy proportional
/// formula (issue #4462).
pub(super) fn legacy_max_input_tokens(window: u64) -> u64 {
    window.saturating_sub(legacy_output_reserve_tokens(window))
}

/// Deterministic history trim that replaces the crate `MessageTrimMiddleware`
/// (issue #4462), restoring three regression guards the crate trim lost:
///
/// 1. **Image-aware token estimate** — inline `[IMAGE:…]` markers and native
///    image blocks are each charged a flat [`IMAGE_MARKER_TOKEN_COST`] instead
///    of their base64 length, so one large image can no longer read as ~2M
///    tokens and evict the whole transcript.
/// 2. **System messages never dropped** — only non-system history is evictable;
///    the crate trim reorders system messages to the front and drops them as a
///    last resort.
/// 3. **Order preserved + observable** — retained messages keep their original
///    relative order, leading orphaned tool results are snapped past (so no
///    provider 400), and any eviction logs a grep-able `warn` carrying
///    (messages dropped, messages/tokens before-and-after).
pub(crate) struct ImageAwareMessageTrimMiddleware {
    /// Input-prompt token budget (already net of the proportional reply reserve).
    budget: u64,
}

impl ImageAwareMessageTrimMiddleware {
    /// Build a trim middleware whose budget is the legacy proportional
    /// [`legacy_max_input_tokens`] for `window` (issue #4462) — NOT the crate's
    /// fixed `window − 16384`. Floored at 1 so the budget is always positive.
    pub(crate) fn for_context_window(window: u64) -> Self {
        Self {
            budget: legacy_max_input_tokens(window).max(1),
        }
    }
}

#[async_trait]
impl Middleware<()> for ImageAwareMessageTrimMiddleware {
    fn name(&self) -> &str {
        "image_aware_message_trim"
    }

    async fn before_model(
        &self,
        _ctx: &mut RunContext<()>,
        _state: &(),
        request: &mut ModelRequest,
    ) -> TaResult<()> {
        let messages = &mut request.messages;
        let original_tokens: u64 = messages.iter().map(estimate_message_tokens).sum();
        if original_tokens <= self.budget {
            return Ok(());
        }
        let original_len = messages.len();

        // Evict oldest non-system messages first, preserving the relative order
        // of every retained message (rebuilding as `system ++ other` would
        // reorder history when a system message appears after non-system ones —
        // exactly the crate-trim regression). System messages are NEVER dropped.
        let mut removable_positions: Vec<usize> = messages
            .iter()
            .enumerate()
            .filter_map(|(idx, m)| (!matches!(m, TaMessage::System(_))).then_some(idx))
            .collect();

        let mut removed = 0usize;
        while !removable_positions.is_empty() {
            let total: u64 = messages.iter().map(estimate_message_tokens).sum();
            if total <= self.budget {
                break;
            }
            let absolute_idx = removable_positions.remove(0);
            // Subsequent positions shift left by one for every prior removal.
            let remove_at = absolute_idx - removed;
            messages.remove(remove_at);
            removed += 1;
        }

        // Snap the window forward past any leading orphaned tool results: dropping
        // an `assistant(tool_calls)` while keeping its `tool` answer leaves the
        // transcript opening on a tool message with no preceding tool-call, which
        // native providers reject with a 400. Drop leading tool results until the
        // first non-system message is a clean turn boundary.
        while let Some(first_non_system) = messages
            .iter()
            .position(|m| !matches!(m, TaMessage::System(_)))
        {
            if matches!(messages[first_non_system], TaMessage::Tool(_)) {
                messages.remove(first_non_system);
                removed += 1;
            } else {
                break;
            }
        }

        if removed > 0 {
            let final_tokens: u64 = messages.iter().map(estimate_message_tokens).sum();
            tracing::warn!(
                messages_dropped = removed,
                messages_before = original_len,
                messages_after = messages.len(),
                tokens_before = original_tokens,
                tokens_after = final_tokens,
                budget = self.budget,
                "[tinyagents::mw] message_trim evicted oldest history to fit the token budget"
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // #4462: image-aware token estimation. A base64 image marker must be priced
    // at the flat IMAGE_MARKER_TOKEN_COST, not chars/4 of its payload — otherwise
    // one image reads as millions of tokens and the trimmer evicts everything,
    // including the system prompt.

    #[test]
    fn estimate_text_tokens_markerless_is_chars_over_four() {
        assert_eq!(estimate_text_tokens(&"a".repeat(40)), (40 + 3) / 4);
        assert_eq!(estimate_text_tokens(""), 0);
    }

    #[test]
    fn estimate_text_tokens_prices_image_marker_flat_not_by_length() {
        let huge = "x".repeat(40_000);
        let text = format!("[IMAGE:{huge}]");
        let tokens = estimate_text_tokens(&text);
        // chars/4 of the payload would be ~10_000; the flat price is 1_200.
        assert!(
            tokens >= IMAGE_MARKER_TOKEN_COST,
            "at least the flat image cost: {tokens}"
        );
        assert!(
            tokens < 2_000,
            "image priced flat, not by base64 length: {tokens}"
        );
    }

    #[test]
    fn estimate_text_tokens_charges_each_image_marker_once() {
        let tokens = estimate_text_tokens("[IMAGE:aaaa] and [IMAGE:bbbb]");
        assert!(
            tokens >= 2 * IMAGE_MARKER_TOKEN_COST,
            "two images each priced: {tokens}"
        );
        assert!(
            tokens < 2 * IMAGE_MARKER_TOKEN_COST + 100,
            "no runaway from the surrounding text: {tokens}"
        );
    }
    use serde_json::json;
    use tinyagents::harness::context::{RunConfig, RunContext};
    use tinyagents::harness::model::ModelRequest;

    fn ctx() -> RunContext<()> {
        RunContext::new(RunConfig::new("mw-test"), ())
    }

    #[tokio::test]
    async fn prompt_cache_segments_fingerprint_full_tool_schema() {
        let mw = PromptCacheSegmentMiddleware;
        let mut first =
            ModelRequest::new(vec![TaMessage::system("sys")]).with_tools(vec![ToolSchema::new(
                "lookup",
                "lookup a user",
                json!({
                    "type": "object",
                    "properties": {
                        "id": { "type": "string" }
                    },
                }),
            )]);
        mw.before_model(&mut ctx(), &(), &mut first).await.unwrap();

        let mut second =
            ModelRequest::new(vec![TaMessage::system("sys")]).with_tools(vec![ToolSchema::new(
                "lookup",
                "lookup a user",
                json!({
                    "type": "object",
                    "properties": {
                        "id": { "type": "integer" }
                    },
                }),
            )]);
        mw.before_model(&mut ctx(), &(), &mut second).await.unwrap();

        let first_tool_segment = first
            .cache_segments
            .iter()
            .find(|segment| segment.role == SegmentRole::Tools)
            .expect("tool segment");
        let second_tool_segment = second
            .cache_segments
            .iter()
            .find(|segment| segment.role == SegmentRole::Tools)
            .expect("tool segment");

        assert_ne!(
            first_tool_segment.id, second_tool_segment.id,
            "same-name tools with different schemas must bust the stable prefix"
        );
        assert_ne!(first.prompt_fingerprint, second.prompt_fingerprint);
        assert_eq!(
            first.prompt_fingerprint.as_deref().unwrap().len(),
            64,
            "request prompt fingerprints use TinyAgents' SHA-256 shape"
        );
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

    #[tokio::test]
    async fn raw_security_policy_block_is_enriched_with_workaround_and_relay() {
        let mw = outcome_capture_mw();
        let mut result = tool_result(
            "run_command",
            "[policy-blocked] Security policy: read-only mode — only read commands are allowed",
        );
        result.error = Some(result.content.clone());
        mw.after_tool(&mut ctx(), &(), &mut result).await.unwrap();
        // The bare denial now carries a workaround + relay directive, and keeps the
        // marker so classification / the loop-breaker still recognise it.
        assert!(result.content.contains("Workaround:"), "{}", result.content);
        assert!(result.content.contains("Relay this to the user"));
        assert!(result
            .content
            .contains(crate::openhuman::security::POLICY_BLOCKED_MARKER));
        assert!(result.content.contains("read-only mode"));
    }

    #[tokio::test]
    async fn already_structured_denial_is_not_double_wrapped() {
        // A ToolPolicyMiddleware-style denial already has "Workaround:"; the capture
        // middleware must leave it untouched (no second Workaround block).
        let mw = outcome_capture_mw();
        let structured =
            "Blocked: Tool 'x' denied. Reason: nope. Workaround: do y. Relay this to the user: ...";
        let mut result = tool_result("x", structured);
        result.error = Some(result.content.clone());
        mw.after_tool(&mut ctx(), &(), &mut result).await.unwrap();
        assert_eq!(
            result.content.matches("Workaround:").count(),
            1,
            "must not double-wrap: {}",
            result.content
        );
    }

    // ── TurnContextMiddleware config ────────────────────────────────────────

    #[test]
    fn defaults_enable_the_byte_cap_only() {
        let mw = TurnContextMiddleware::defaults();
        assert_eq!(
            mw.tool_result_budget_bytes,
            DEFAULT_TOOL_RESULT_BUDGET_BYTES
        );
        assert!(mw.payload_summarizer.is_none());
        assert_eq!(mw.microcompact_keep_recent, 0);
        // Autocompaction defaults on (channel/sub-agent); the chat path overrides
        // it from config.
        assert!(mw.autocompact_enabled);
        // The byte cap alone is enough to make the bundle non-empty (CacheAlign
        // was deleted in C3, so it no longer contributes here).
        assert!(!mw.is_empty());
    }

    #[test]
    fn an_all_default_bundle_installs_nothing() {
        assert!(TurnContextMiddleware::default().is_empty());
    }

    #[test]
    fn tokenjuice_only_bundle_is_not_empty() {
        let mw = TurnContextMiddleware {
            tokenjuice_compaction_enabled: true,
            tokenjuice_compression: AgentTokenjuiceCompression::Light,
            ..Default::default()
        };
        assert!(!mw.is_empty());
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

    // ── MicrocompactMiddleware (crate) ──────────────────────────────────────
    //
    // These assert the crate `MicrocompactMiddleware`, constructed with
    // OpenHuman's `CLEARED_PLACEHOLDER`, reproduces the deleted in-house
    // middleware byte-for-byte — the parity contract for the upstream swap.

    #[tokio::test]
    async fn microcompact_clears_older_tool_bodies_and_keeps_recent() {
        let mw = MicrocompactMiddleware::new(1, CLEARED_PLACEHOLDER);
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
        let mw = MicrocompactMiddleware::new(5, CLEARED_PLACEHOLDER);
        let mut req =
            ModelRequest::new(vec![TaMessage::tool("t1", "A"), TaMessage::tool("t2", "B")]);
        mw.before_model(&mut ctx(), &(), &mut req).await.unwrap();
        assert_eq!(req.messages[0].text(), "A");
        assert_eq!(req.messages[1].text(), "B");
    }

    #[tokio::test]
    async fn microcompact_is_idempotent() {
        let mw = MicrocompactMiddleware::new(1, CLEARED_PLACEHOLDER);
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
            artifact_store: None,
            tokenjuice_compaction_enabled: false,
            tokenjuice_compression: AgentTokenjuiceCompression::Off,
            tool_policies: HashMap::new(),
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
            artifact_store: None,
            tokenjuice_compaction_enabled: false,
            tokenjuice_compression: AgentTokenjuiceCompression::Off,
            tool_policies: HashMap::new(),
        };
        let mut result = tool_result("echo", "tiny");
        mw.after_tool(&mut ctx(), &(), &mut result).await.unwrap();
        assert_eq!(result.content, "tiny");
    }

    #[test]
    fn tool_char_cap_reads_the_tools_own_declared_cap() {
        let mut tool_policies = HashMap::new();
        tool_policies.insert(
            "big".to_string(),
            TaToolPolicy::classified().with_runtime(tinyagents::harness::tool::ToolRuntime {
                timeout_ms: None,
                timeout: tinyagents::harness::tool::ToolTimeout::Inherit,
                max_retries: None,
                idempotent: false,
                cancelable: true,
                sandbox: tinyagents::harness::tool::SandboxMode::Inherit,
                max_result_bytes: Some(10),
                streaming: false,
            }),
        );
        let mw = ToolOutputMiddleware {
            budget_bytes: 1_000,
            payload_summarizer: None,
            artifact_store: None,
            tokenjuice_compaction_enabled: false,
            tokenjuice_compression: AgentTokenjuiceCompression::Off,
            tool_policies,
        };
        // Tool declares its own char cap → surfaced for the per-tool truncation.
        assert_eq!(mw.tool_char_cap("big"), Some(10));
        // Unknown tool → no per-tool cap (the flat byte budget applies instead).
        assert_eq!(mw.tool_char_cap("other"), None);
    }

    #[tokio::test]
    async fn tool_output_honors_a_tools_own_cap() {
        let mut tool_policies = HashMap::new();
        tool_policies.insert(
            "capped".to_string(),
            TaToolPolicy::classified().with_runtime(tinyagents::harness::tool::ToolRuntime {
                timeout_ms: None,
                timeout: tinyagents::harness::tool::ToolTimeout::Inherit,
                max_retries: None,
                idempotent: false,
                cancelable: true,
                sandbox: tinyagents::harness::tool::SandboxMode::Inherit,
                max_result_bytes: Some(20),
                streaming: false,
            }),
        );
        let mw = ToolOutputMiddleware {
            budget_bytes: 100_000,
            payload_summarizer: None,
            artifact_store: None,
            tokenjuice_compaction_enabled: false,
            tokenjuice_compression: AgentTokenjuiceCompression::Off,
            tool_policies,
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

    // ── CostBudgetMiddleware ────────────────────────────────────────────────

    #[tokio::test]
    async fn cost_budget_is_a_noop_without_a_global_tracker() {
        // No global CostTracker is installed in the unit-test process, so the
        // gate self-disables and the model call proceeds.
        let mw = CostBudgetMiddleware::new();
        let mut req = ModelRequest::new(vec![TaMessage::user("hi")]);
        assert!(mw.before_model(&mut ctx(), &(), &mut req).await.is_ok());
    }

    // ── CostBudgetMiddleware shadow (W2-budget-dedupe) ──────────────────────

    /// The shadow comparison at `after_agent` logs parity when the crate
    /// `BudgetMiddleware`'s tracker matches the runtime `AgentRun.usage`, and
    /// never fails the run — in both the matching and diverging cases. It also
    /// must be inert (no panic, `Ok`) when no shadow tracker is installed.
    #[tokio::test]
    async fn cost_budget_shadow_after_agent_never_fails_the_run() {
        use tinyagents::harness::usage::Usage;

        // No shadow tracker: after_agent is a silent no-op.
        let plain = CostBudgetMiddleware::new();
        let mut run = AgentRun::new();
        run.usage.record(Usage::new(100, 40));
        assert!(plain.after_agent(&mut ctx(), &(), &mut run).await.is_ok());

        // Matching tracker (parity): the crate tracker accumulated the same
        // single call's usage the runtime recorded into `run.usage`.
        let tracker = BudgetTracker::new();
        tracker.record(Usage::new(100, 40), Default::default());
        let shadow = CostBudgetMiddleware::with_shadow(tracker.clone());
        let mut run = AgentRun::new();
        run.usage.record(Usage::new(100, 40));
        assert!(shadow.after_agent(&mut ctx(), &(), &mut run).await.is_ok());

        // Diverging tracker (crate missed a call): still only logs, never fails.
        let mut diverged_run = AgentRun::new();
        diverged_run.usage.record(Usage::new(100, 40));
        diverged_run.usage.record(Usage::new(10, 5));
        assert!(shadow
            .after_agent(&mut ctx(), &(), &mut diverged_run)
            .await
            .is_ok());
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

    #[tokio::test]
    async fn repeated_tool_failure_pauses_only_after_the_threshold() {
        let handle = SteeringHandle::allow_all();
        let mw = RepeatedToolFailureMiddleware::new(
            handle.clone(),
            3,
            std::sync::Arc::new(std::sync::Mutex::new(None)),
        );
        // Two identical failures: below the halt threshold. The crate ladder
        // nudges (Redirect) on the second, but must NOT pause (halt) yet.
        for _ in 0..2 {
            let mut r = failing_result("flaky", "boom");
            mw.after_tool(&mut ctx(), &(), &mut r).await.unwrap();
        }
        assert_eq!(
            drain_pause_count(&handle),
            0,
            "no halt before the threshold"
        );
        // Third identical failure exhausts the same-strategy retries → halt.
        let mut r = failing_result("flaky", "boom");
        mw.after_tool(&mut ctx(), &(), &mut r).await.unwrap();
        assert_eq!(
            drain_pause_count(&handle),
            1,
            "the third identical failure should pause (halt) the run"
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
        // Two more failures — still below the halt threshold because the counter
        // reset, so the ladder never reaches the third identical repeat.
        for _ in 0..2 {
            let mut r = failing_result("t", "boom");
            mw.after_tool(&mut ctx(), &(), &mut r).await.unwrap();
        }
        assert_eq!(
            drain_pause_count(&handle),
            0,
            "a success should reset the breaker so it never halts"
        );
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
        // deterministic failure loop does (and the varied-failure backstop nudges
        // at 4 / halts at 6, both above this count).
        for err in ["e1", "e2", "e3"] {
            let mut r = failing_result("t", err);
            mw.after_tool(&mut ctx(), &(), &mut r).await.unwrap();
        }
        assert_eq!(
            handle.pending(),
            0,
            "distinct errors below the backstop must not steer the run"
        );
    }

    #[test]
    fn user_actionable_escalation_detects_missing_connection() {
        // A not-connected blocker → a user-directed ask with a concrete next step.
        let ask = user_actionable_escalation(
            "gmail_send",
            "Gmail is not connected. Ask the user to connect 'gmail' in Settings → Connections.",
        )
        .expect("a missing-connection failure is user-actionable");
        assert!(ask.contains("without your input"));
        assert!(ask.contains("Settings"));
        assert!(ask.to_lowercase().contains("connect"));
        assert!(ask.contains("gmail_send"));
        // The original tool text is relayed so the user sees which service.
        assert!(ask.to_lowercase().contains("gmail"));

        // A plain environment failure is NOT user-actionable → keep crate summary.
        assert!(user_actionable_escalation("read_file", "file not found").is_none());
        assert!(user_actionable_escalation("shell", "exit code 1: segfault").is_none());
        assert!(user_actionable_escalation(
            "gmail_send",
            "[composio:error:insufficient_scope] `gmail_send` was rejected because the connected \
             gmail account is missing required permissions (insufficient authentication scopes). \
             Reconnect the integration in Settings → Connections → gmail and grant the scopes \
             requested during OAuth."
        )
        .is_none());
        assert!(user_actionable_escalation(
            "gmail_trigger",
            "[composio:error:trigger_permission] Couldn't enable this trigger: the connected \
             gmail account doesn't have permission to manage triggers. Reconnect gmail in \
             Settings → Connections → gmail and grant the permissions requested during OAuth, \
             then try again."
        )
        .is_none());
    }

    #[tokio::test]
    async fn halt_on_missing_connection_asks_the_user_instead_of_reporting_back() {
        // #4092: a repeated not-connected failure halts with a user-directed ask,
        // not the crate's generic "unreachable environment, report this back".
        let handle = SteeringHandle::allow_all();
        let slot = std::sync::Arc::new(std::sync::Mutex::new(None));
        let mw = RepeatedToolFailureMiddleware::new(handle.clone(), 3, slot.clone());
        // Three identical not-connected failures → halt.
        for _ in 0..3 {
            let mut r = failing_result(
                "slack_post",
                "Slack is not connected — connect it in Settings → Connections.",
            );
            mw.after_tool(&mut ctx(), &(), &mut r).await.unwrap();
        }
        let summary = slot
            .lock()
            .unwrap()
            .clone()
            .expect("halt records a summary");
        assert!(
            summary.contains("without your input") && summary.contains("Settings"),
            "the halt should ask the user to connect the service: {summary}"
        );
        assert!(
            !summary.contains("Report this back"),
            "a user-actionable blocker must not use the generic report-back summary: {summary}"
        );
        assert_eq!(
            drain_pause_count(&handle),
            1,
            "it still pauses the run to surface the ask"
        );
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

    #[tokio::test]
    async fn repeated_tool_failure_nudges_change_of_strategy_before_the_halt() {
        use crate::openhuman::tinyagents::orchestration::{
            openhuman_steering_handle, SteeringRunClass,
        };
        use tinyagents::harness::steering::SteeringCommandKind;

        // #4089: before the same-strategy retry cap, the breaker must feed a
        // structured "no progress since step X" corrective back into the loop so
        // the model changes approach rather than retrying the identical failing
        // call — and it must do so *without* pausing yet.
        let handle = SteeringHandle::allow_all();
        let mw = RepeatedToolFailureMiddleware::new(
            handle.clone(),
            3,
            std::sync::Arc::new(std::sync::Mutex::new(None)),
        );
        // First identical failure: not a loop yet — no steering.
        let mut r = failing_result("read_file", "file not found");
        mw.after_tool(&mut ctx(), &(), &mut r).await.unwrap();
        assert!(
            handle.drain().is_empty(),
            "a single failure is never a loop"
        );
        // Second identical failure: the nudge fires, still no halt.
        let mut r = failing_result("read_file", "file not found");
        mw.after_tool(&mut ctx(), &(), &mut r).await.unwrap();
        let nudges = drain_nudge_messages(&handle);
        assert_eq!(
            nudges.len(),
            1,
            "the repeat should steer the model to change strategy before the retry cap"
        );
        let nudge = &nudges[0];
        assert!(
            nudge.contains("no progress"),
            "the nudge carries the structured no-progress signal: {nudge}"
        );
        assert!(
            nudge.to_lowercase().contains("read_file"),
            "the nudge names the failing call so the model knows what not to repeat: {nudge}"
        );

        // Regression for the #4473 crash: the nudge must ride a steering lane the
        // user's *interactive* turn permits. `Redirect` is Background-only, so a
        // Redirect nudge aborted interactive turns; `InjectMessage` is permitted
        // on both classes. Assert the interactive policy accepts the lane we use.
        let interactive = openhuman_steering_handle(SteeringRunClass::Interactive);
        assert!(
            interactive
                .policy()
                .is_allowed(SteeringCommandKind::InjectMessage),
            "the no-progress nudge must use a lane the interactive turn permits"
        );
        assert!(
            !interactive
                .policy()
                .is_allowed(SteeringCommandKind::Redirect),
            "sanity: interactive still refuses Redirect (the lane that crashed it)"
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
        };
        mw.before_tool(&mut ctx(), &(), &mut call).await.unwrap();
        let mut result = tool_result(name, content); // call_id "c1" matches
        result.error = error.map(|e| e.to_string());
        mw.after_tool(&mut ctx(), &(), &mut result).await.unwrap();
        result
    }

    #[tokio::test]
    async fn memory_write_without_index_read_gets_a_corrective_note() {
        let mw = MemoryProtocolMiddleware::new();
        let result = run_cycle(&mw, "memory_store", json!({}), "stored entry 42", None).await;
        assert!(
            result.content.contains(MEMORY_PROTOCOL_MARKER),
            "a write with no preceding dedupe read should be annotated: {}",
            result.content
        );
        assert!(result
            .content
            .contains("without first reading the memory index"));
        assert!(result.content.contains("update_memory_md"));
        // The original tool output is preserved, guidance is appended.
        assert!(result.content.starts_with("stored entry 42"));
    }

    #[tokio::test]
    async fn full_cycle_read_then_write_then_update_only_reminds_on_the_write() {
        let mw = MemoryProtocolMiddleware::new();

        let read = run_cycle(&mw, "memory_recall", json!({}), "no dupes", None).await;
        assert!(
            !read.content.contains(MEMORY_PROTOCOL_MARKER),
            "a read is not annotated"
        );

        let write = run_cycle(&mw, "memory_store", json!({}), "stored", None).await;
        assert!(write.content.contains(MEMORY_PROTOCOL_MARKER));
        // The read preceded the write, so no missing-read complaint — just the
        // forward "sync the index" reminder.
        assert!(!write
            .content
            .contains("without first reading the memory index"));

        let update = run_cycle(
            &mw,
            "update_memory_md",
            json!({ "file": "MEMORY.md" }),
            "index updated",
            None,
        )
        .await;
        assert!(
            !update.content.contains(MEMORY_PROTOCOL_MARKER),
            "closing the cycle needs no guidance"
        );
    }

    #[tokio::test]
    async fn skill_md_update_does_not_close_the_memory_cycle() {
        let mw = MemoryProtocolMiddleware::new();
        run_cycle(&mw, "memory_recall", json!({}), "checked", None).await;
        run_cycle(&mw, "memory_store", json!({}), "stored", None).await;
        // update_memory_md targeting SKILL.md must NOT reconcile the MEMORY.md
        // index, so the stale-index warning is still owed at run end.
        run_cycle(
            &mw,
            "update_memory_md",
            json!({ "file": "SKILL.md" }),
            "skill updated",
            None,
        )
        .await;
        let mut run = AgentRun::new();
        // Still pending → after_agent takes its warn path without erroring.
        mw.after_agent(&mut ctx(), &(), &mut run).await.unwrap();
        // A following write reports drift, proving pending was not cleared.
        let next = run_cycle(&mw, "memory_store", json!({}), "again", None).await;
        assert!(
            next.content.contains("drifting"),
            "SKILL.md update must not mask the stale MEMORY.md index: {}",
            next.content
        );
    }

    #[tokio::test]
    async fn consolidated_memory_tree_ingest_is_treated_as_a_write() {
        let mw = MemoryProtocolMiddleware::new();
        let ingest = run_cycle(
            &mw,
            "memory_tree",
            json!({ "mode": "ingest_document" }),
            "ingested",
            None,
        )
        .await;
        assert!(
            ingest.content.contains(MEMORY_PROTOCOL_MARKER),
            "memory_tree ingest_document is a write and must be annotated: {}",
            ingest.content
        );
    }

    #[tokio::test]
    async fn failed_memory_write_does_not_advance_the_protocol() {
        let mw = MemoryProtocolMiddleware::new();
        let failed = run_cycle(
            &mw,
            "memory_store",
            json!({}),
            "disk full",
            Some("disk full"),
        )
        .await;
        // A failed write is not annotated and leaves nothing pending, so a later
        // run-end sweep must not warn about a stale index.
        assert!(!failed.content.contains(MEMORY_PROTOCOL_MARKER));
        let mut run = AgentRun::new();
        // after_agent is a no-op warn path; it must not error.
        mw.after_agent(&mut ctx(), &(), &mut run).await.unwrap();
    }

    #[tokio::test]
    async fn second_write_without_an_update_flags_index_drift() {
        let mw = MemoryProtocolMiddleware::new();
        run_cycle(&mw, "memory_recall", json!({}), "checked", None).await;
        let first = run_cycle(&mw, "memory_store", json!({}), "a", None).await;
        assert!(!first.content.contains("drifting"));

        // No update_memory_md between the two writes → the index is drifting.
        let second = run_cycle(&mw, "memory_store", json!({}), "b", None).await;
        assert!(
            second.content.contains("drifting"),
            "a second unsynced write should flag index drift: {}",
            second.content
        );
    }
}
