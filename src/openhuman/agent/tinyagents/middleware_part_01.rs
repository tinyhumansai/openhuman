use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use sha2::{Digest, Sha256};

use tinyagents_harness::error::Result as TaResult;
use tinyagents_harness::context::RunContext;
use tinyagents_harness::events::AgentEvent;
use tinyinference::message::{ContentBlock, Message as TaMessage};
use tinyagents_harness::middleware::{
    AgentRun, BudgetTracker, ContextualToolSelectionMiddleware, Middleware,
    MiddlewareToolOutcome, ToolAllowlistMiddleware, ToolHandler, ToolMiddleware,
};
use tinyinference::model::{ModelRequest, ModelResponse, PromptSegment, SegmentRole};
use tinyagents_harness::no_progress::{
    NoProgress, NoProgressTracker, SuccessfulRepeat, SuccessfulRepeatTracker, ToolAttempt,
};
use tinyagents_harness::runtime::AgentHarness;
use tinyagents_harness::steering::{SteeringCommand, SteeringHandle};
use tinyagents_harness::tool::{ToolPolicy as TaToolPolicy, ToolResult as TaToolResult};
use tinyinference::tool::{ToolCall as TaToolCall, ToolSchema};

use crate::openhuman::agent::context::CLEARED_PLACEHOLDER;
use crate::openhuman::agent::harness::tool_result_artifacts::{
    apply_per_result_persistence, ToolResultArtifactStore, TINYAGENTS_TOOL_RESULT_ARTIFACT_STORE,
};
use crate::openhuman::agent::tinyagents::payload_summarizer::{
    PayloadSummarizer, SummarizeOutcome, UnavailableReason,
};
use crate::openhuman::inference::tokenjuice::AgentTokenjuiceCompression;
use crate::openhuman::security::approval::{
    redact_args, summarize_action, ApprovalGate, ExecutionOutcome, GateOutcome,
};
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
    Arc<std::sync::Mutex<Vec<crate::openhuman::agent::messages::ChatMessage>>>;

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
        let history =
            crate::openhuman::agent::message_convert::messages_to_history(&request.messages);
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
        // Microcompact is NOT registered here any more (issue #6014). It used to
        // be, which put its `before_model` ahead of the summarization step the
        // caller installs later — so by the time the task-aware summarizer ran,
        // every tool body past `keep_recent` had already been replaced with
        // `CLEARED_PLACEHOLDER` and it was summarizing placeholders. The one
        // component able to preserve those results in condensed form never saw
        // them. The caller now sites it AFTER compression, so the ladder reads
        // summarize → blank → evict. See `assemble_turn_harness`.
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
/// [`CacheLayoutEvent`](tinyagents_harness::cache::CacheLayoutEvent). This is
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

/// Tools whose results are self-describing JSON payloads that downstream
/// extractors and the frontend canvas parse structurally (the `type` marker
/// must survive). Compacting/summarizing them destroys the contract and
/// serves no purpose — the model doesn't benefit from a tabulated graph and
/// the payload is the turn's final output, not intermediate context.
///
/// These tools are exempt from *every* content-rewriting stage below —
/// tokenjuice compaction (steps 1+2) **and** the per-tool char cap / shared
/// byte-budget backstop (steps 3+4, see [`is_truncation_exempt`]). Both
/// `flows::ops::extract_workflow_proposal` and the frontend's
/// `parseWorkflowProposal` parse this content as a single whole-string JSON
/// document; a byte-cap truncation at a UTF-8 boundary produces invalid JSON
/// just as surely as tokenjuice tabulation strips the `"type"` marker — both
/// end in a silent `proposal: None` and a blank canvas. A ≥10-node graph
/// routinely clears the ~16 KiB shared budget, so the truncation exemption
/// matters just as much as the compaction one.
const COMPACTION_EXEMPT_TOOLS: &[&str] = &[
    "propose_workflow",
    "revise_workflow",
    "edit_workflow",
    "save_workflow",
    "create_workflow",
];

/// Tools whose results the model reads to derive an exact schema (e.g.
/// `primary_array_path` / `output_fields`) from a *real* sampled tool
/// response, per the B12 output-probe contract (`flows::builder_tools`).
/// TokenJuice's array-elision tabulation defeats their purpose outright — a
/// tabulated sample hides the very array shape the model is calling the tool
/// to observe, so it derives a wrong or nonexistent `split_out.path` from the
/// summary instead of the real response. They're compaction-exempt
/// ([`is_compaction_exempt`]) for that reason.
///
/// Unlike [`COMPACTION_EXEMPT_TOOLS`], their payload is intermediate context
/// the model reasons over — not the turn's final machine-parsed output — and
/// samples can be genuinely large (a full API response body). So they stay
/// subject to the per-tool char cap / shared byte-budget backstop
/// ([`is_truncation_exempt`] returns `false` for them): a truncated-but-not-
/// tabulated sample is still a usable (if partial) real response, and the
/// backstop keeps these calls from blowing the context budget.
const SAMPLING_TOOLS: &[&str] = &["get_tool_output_sample", "get_tool_contract"];

/// Steps 1 (payload summarizer) + 2 (tokenjuice compaction) exemption:
/// proposal tools (final-output contract, see [`COMPACTION_EXEMPT_TOOLS`])
/// plus sampling tools (tabulation would corrupt the schema they exist to
/// reveal, see [`SAMPLING_TOOLS`]).
fn is_compaction_exempt(name: &str) -> bool {
    COMPACTION_EXEMPT_TOOLS.contains(&name) || SAMPLING_TOOLS.contains(&name)
}

/// Steps 3 (per-tool char cap) + 4 (shared byte-budget backstop) exemption:
/// proposal tools only. Their JSON is parsed as a single whole-string
/// document downstream, so any truncation — not just tokenjuice tabulation —
/// breaks the parse. Sampling tools are deliberately *not* in this set: see
/// [`SAMPLING_TOOLS`] for why the byte cap stays in force for them.
fn is_truncation_exempt(name: &str) -> bool {
    COMPACTION_EXEMPT_TOOLS.contains(&name)
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
