//! Oversized-tool-result compression via the `summarizer` sub-agent.
//!
//! ## The problem
//!
//! When the orchestrator calls a tool that returns a huge payload — a
//! Composio action dumping 200 KB of JSON, a web scrape returning 50 KB
//! of markdown, a `file_read` spitting back a multi-thousand-line log —
//! the raw blob lands verbatim in the orchestrator's history and burns
//! context budget. The only existing guardrail is
//! [`crate::openhuman::config::ContextConfig::tool_result_budget_bytes`],
//! which hard-truncates mid-payload, dropping whatever happens to be
//! past the cut.
//!
//! ## The fix
//!
//! This module routes oversized tool results through a dedicated
//! `summarizer` sub-agent (model hint `"summarization"`) before they
//! enter agent history. The summarizer compresses the payload per an
//! extraction contract that preserves identifiers and key facts, and
//! the compressed summary is what the parent agent sees. Truncation
//! remains the final backstop downstream when summarization fails or
//! the payload is so absurdly large that paying for an LLM call on it
//! makes no economic sense.
//!
//! ## Trigger conditions
//!
//! [`PayloadSummarizer::maybe_summarize_in_parent`] leaves the raw payload in
//! place when:
//!
//! * The raw payload is below
//!   [`SubagentPayloadSummarizer::threshold_tokens`] (config default 4 000
//!   tokens — small payloads aren't worth an extra LLM round-trip).
//!   Token count is estimated as `chars / 4`, matching
//!   `tree_summarizer::estimate_tokens`.
//! * The raw payload is above
//!   [`SubagentPayloadSummarizer::max_payload_tokens`] (default
//!   2 000 000 tokens — too big to summarize cost-effectively; existing
//!   `tool_result_budget_bytes` truncation handles it instead).
//! * The internal failure circuit-breaker has tripped (3 consecutive
//!   sub-agent failures within the same session disable summarization
//!   for the rest of the session, so a broken summarizer can't tank
//!   every tool call).
//! * The sub-agent dispatch returns an error or an empty / non-shrinking
//!   summary — pass-through preserves the raw payload as a safety net.
//!
//! ## Scope
//!
//! Only the orchestrator session gets a `PayloadSummarizer` wired in
//! ([`crate::openhuman::agent::harness::session::builder::AgentBuilder`]
//! checks `agent_id == "orchestrator"`). Welcome, integrations_agent,
//! researcher, planner, archivist, and every other typed sub-agent get
//! `None` and their tool results are untouched. The summarizer itself
//! is also `None` so it can never recursively summarize its own input.

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use tinyagents_harness::context::RunContext;
use tinyagents_harness::runtime::{AgentHarness, InvalidArgsPolicy, RunPolicy, UnknownToolPolicy};
use tinyagents_harness::subagent::SubAgent;
use tracing::{debug, info, warn};

use crate::openhuman::agent::harness::definition::{AgentDefinition, PromptSource};
use crate::openhuman::agent::harness::fork_context::{current_parent, ParentExecutionContext};
use crate::openhuman::agent::harness::subagent_runner;

/// A successful compression, carried by [`SummarizeOutcome::Summarized`].
///
/// The caller replaces the raw payload with [`SummarizedPayload::summary`]
/// before appending it to agent history.
#[derive(Debug, Clone)]
pub struct SummarizedPayload {
    /// The compressed summary text. Replaces the raw tool output.
    pub summary: String,
    /// Original payload size in bytes — for logging/observability.
    pub original_bytes: usize,
    /// Compressed summary size in bytes — for logging/observability.
    pub summary_bytes: usize,
}

/// Why a payload reached agent history without being summarized.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnavailableReason {
    /// Larger than `summarizer_max_payload_tokens`, so summarization was never
    /// attempted and the payload will be truncated downstream.
    PayloadTooLarge,
    /// The failure circuit breaker is open for the rest of this session.
    Disabled,
    /// The summarizer sub-agent ran and did not produce a usable summary.
    Failed,
}

impl UnavailableReason {
    /// The model-facing notice for this reason.
    ///
    /// **Prefixed** to the tool result, never appended, and prefixed by the
    /// caller *after* its truncation stages have run. Both halves matter. The
    /// per-tool char cap keeps the head (`content.chars().take(cap)`), so an
    /// appended notice is the first thing truncation removes — but a notice
    /// prefixed *before* that cap is no safer, because a tool declaring a
    /// `max_result_size_chars` below this string's length truncates the notice
    /// itself. Applying it last is the only placement that survives both.
    ///
    /// Every variant ends with the same instruction: *do not re-run the tool
    /// for a summary*. Without it, the model's reasonable response to a
    /// truncated dump is to call the same tool again — the silent re-dispatch
    /// loop that reads to a user as a hang.
    ///
    /// It is a bare **instruction**, with no justifying clause, and that is
    /// deliberate. Three attempts to justify it were all false, each in a
    /// different direction:
    ///
    /// - *"Re-running this tool will return the same result."* Untrue for any
    ///   API-backed tool, which is time-varying, and it suppresses a retry the
    ///   model may have had good reason to make.
    /// - *"Re-running it will not produce a summary."* Untrue for [`Self::Failed`]
    ///   specifically: that variant is recorded before the breaker opens, so a
    ///   later attempt can genuinely succeed. Saying otherwise contradicts the
    ///   breaker's own behaviour two paragraphs up.
    /// - *"…the full output is already here."* Untrue whenever a cap fired.
    ///   This notice is applied *after* the per-tool and byte-budget stages
    ///   precisely so it survives them, which means the payload beneath it may
    ///   be truncated — and [`Self::PayloadTooLarge`] says "may be truncated"
    ///   in the same breath, so that pairing contradicted itself outright.
    ///
    /// The variant-specific reason already sits in the first half of each
    /// notice, and it is a fact about the summarizer rather than a prediction
    /// about the tool. The instruction needs no second reason, and every
    /// candidate for one has turned out to be a claim this code cannot make.
    #[must_use]
    pub fn notice(self) -> &'static str {
        match self {
            Self::PayloadTooLarge => concat!(
                "[openhuman: summarization unavailable — this output exceeds the summarizer's ",
                "size cap, so the tool output follows and may be truncated. ",
                "Do not re-run the tool for a summary.]"
            ),
            Self::Disabled => concat!(
                "[openhuman: summarization unavailable — it is switched off for this session ",
                "after repeated failures, so the tool output follows. ",
                "Do not re-run the tool for a summary.]"
            ),
            Self::Failed => concat!(
                "[openhuman: summarization unavailable — the summarizer did not return a usable ",
                "summary for this result, so the tool output follows. ",
                "Do not re-run the tool for a summary.]"
            ),
        }
    }
}

/// What one summarization attempt concluded.
///
/// Replaces a bare `Option<SummarizedPayload>`, in which `None` meant both
/// "nothing to do" and "this failed". That conflation was the defect: the one
/// production caller could not tell the two apart, so a failed summarization
/// entered agent history looking like ordinary tool output.
#[derive(Debug, Clone)]
pub enum SummarizeOutcome {
    /// Replace the raw payload with this summary.
    Summarized(SummarizedPayload),
    /// The payload did not need summarizing. Keep it and say nothing — a
    /// notice here would be noise on every small tool result.
    NotNeeded,
    /// The raw payload is what the model will see. Keep it, and prefix
    /// [`UnavailableReason::notice`] so the model knows why.
    Unavailable(UnavailableReason),
}

/// Trait for anything that can compress a tool result before it enters
/// agent history. Implementations decide the threshold, the dispatch
/// mechanism, and the failure policy.
#[async_trait]
pub trait PayloadSummarizer: Send + Sync {
    /// TinyAgents parent-context-aware entry point.
    ///
    /// See [`SummarizeOutcome`]. The three states are deliberately distinct:
    /// a caller must be able to tell "this payload was fine as it was" from
    /// "summarization did not happen", because only the second needs to be
    /// disclosed to the model.
    ///
    /// A failed summarization should still never break a tool call, so
    /// implementations report failure as
    /// [`SummarizeOutcome::Unavailable`] rather than `Err`. `Err` remains for
    /// fatal misconfiguration, and the caller now handles it rather than
    /// pattern-matching it away.
    async fn maybe_summarize_in_parent(
        &self,
        parent_ctx: &RunContext<()>,
        tool_name: &str,
        parent_task_hint: Option<&str>,
        raw: &str,
    ) -> Result<SummarizeOutcome>;
}

/// Default implementation that dispatches the `summarizer` through
/// TinyAgents [`SubAgent::invoke_in_parent`].
///
/// Holds the `summarizer` agent definition (resolved once at agent
/// build time from the global
/// [`crate::openhuman::agent::harness::definition::AgentDefinitionRegistry`])
/// plus the threshold knobs and a small failure counter that acts as a
/// session-scoped circuit breaker.
pub struct SubagentPayloadSummarizer {
    /// The `summarizer` agent definition. Cloned from the registry at
    /// agent build time so the runner doesn't have to re-resolve it
    /// per call.
    definition: AgentDefinition,
    /// Lower bound, in **estimated tokens** (`chars / 4`): tool results
    /// smaller than this are passed through untouched. Default is
    /// `summarizer_payload_threshold_tokens` from
    /// [`crate::openhuman::config::ContextConfig`] (default 4 000 tokens).
    threshold_tokens: usize,
    /// Upper bound, in **estimated tokens**: tool results larger than
    /// this are also passed through (no LLM call) and fall through to
    /// the existing `tool_result_budget_bytes` truncation downstream.
    /// Default is `summarizer_max_payload_tokens` from
    /// [`crate::openhuman::config::ContextConfig`] (2 000 000 tokens).
    max_payload_tokens: usize,
    /// Consecutive failure count. Reset to zero on any successful
    /// summarization. Once it reaches
    /// [`Self::max_failures_before_disable`] the circuit breaker
    /// trips and the summarizer becomes a no-op for the rest of the
    /// session.
    failures: Arc<Mutex<u8>>,
    /// Number of consecutive failures that disables the summarizer
    /// for the rest of the session. Hardcoded to 3 — a misbehaving
    /// summarizer should not silently degrade every tool call.
    max_failures_before_disable: u8,
}

impl SubagentPayloadSummarizer {
    /// Build a new summarizer wrapping the given definition and limits.
    ///
    /// `threshold_tokens` and `max_payload_tokens` are both in
    /// estimated tokens (`chars / 4`).
    pub fn new(
        definition: AgentDefinition,
        threshold_tokens: usize,
        max_payload_tokens: usize,
    ) -> Self {
        Self {
            definition,
            threshold_tokens,
            max_payload_tokens,
            failures: Arc::new(Mutex::new(0)),
            max_failures_before_disable: 3,
        }
    }

    /// Has the failure circuit breaker tripped?
    fn breaker_tripped(&self) -> bool {
        match self.failures.lock() {
            Ok(g) => *g >= self.max_failures_before_disable,
            // If the mutex is poisoned, fail safe by treating the
            // breaker as tripped — a poisoned mutex means a previous
            // panic, and a panic during summarization is itself a
            // good reason to stop trying.
            Err(_) => true,
        }
    }

    /// Increment the consecutive-failure counter.
    fn record_failure(&self) {
        if let Ok(mut g) = self.failures.lock() {
            *g = g.saturating_add(1);
            if *g == self.max_failures_before_disable {
                warn!(
                    "[payload_summarizer] circuit breaker tripped after {} consecutive failures — disabling for session",
                    self.max_failures_before_disable
                );
            }
        }
    }

    /// Reset the consecutive-failure counter on a clean run.
    fn record_success(&self) {
        if let Ok(mut g) = self.failures.lock() {
            *g = 0;
        }
    }
}

#[async_trait]
impl PayloadSummarizer for SubagentPayloadSummarizer {
    async fn maybe_summarize_in_parent(
        &self,
        parent_ctx: &RunContext<()>,
        tool_name: &str,
        parent_task_hint: Option<&str>,
        raw: &str,
    ) -> Result<SummarizeOutcome> {
        let tokens = estimate_tokens(raw);

        // ── 1. Pass-through checks ─────────────────────────────────────
        if tokens < self.threshold_tokens {
            debug!(
                tool = tool_name,
                tokens = tokens,
                bytes = raw.len(),
                threshold = self.threshold_tokens,
                "[payload_summarizer] below threshold, passing through"
            );
            // The only genuinely uneventful exit: the payload was fine as it
            // was, so the model is told nothing.
            return Ok(SummarizeOutcome::NotNeeded);
        }
        if tokens > self.max_payload_tokens {
            warn!(
                tool = tool_name,
                tokens = tokens,
                bytes = raw.len(),
                max = self.max_payload_tokens,
                "[payload_summarizer] payload exceeds max cap, skipping summarization (will be truncated downstream)"
            );
            return Ok(SummarizeOutcome::Unavailable(
                UnavailableReason::PayloadTooLarge,
            ));
        }
        if self.breaker_tripped() {
            warn!(
                tool = tool_name,
                tokens = tokens,
                bytes = raw.len(),
                "[payload_summarizer] circuit breaker tripped, skipping summarization"
            );
            return Ok(SummarizeOutcome::Unavailable(UnavailableReason::Disabled));
        }

        info!(
            tool = tool_name,
            tokens = tokens,
            bytes = raw.len(),
            parent_depth = parent_ctx.depth(),
            "[payload_summarizer] dispatching summarizer via tinyagents parent context"
        );

        let prompt = build_summarizer_prompt(tool_name, parent_task_hint, raw);
        let started = std::time::Instant::now();
        let outcome = self
            .invoke_tinyagents_summarizer_in_parent(parent_ctx, prompt)
            .await;
        self.handle_summarizer_result(tool_name, raw, started, outcome)
    }
}

impl SubagentPayloadSummarizer {
    async fn invoke_tinyagents_summarizer_in_parent(
        &self,
        parent_ctx: &RunContext<()>,
        prompt: String,
    ) -> Result<String> {
        let parent = current_parent().ok_or_else(|| {
            anyhow!("payload summarizer cannot use invoke_in_parent without ParentExecutionContext")
        })?;
        let config_loaded = crate::openhuman::config::Config::load_or_init().await;
        let (source, model) = subagent_runner::resolve_subagent_source(
            &self.definition.model,
            &self.definition.id,
            config_loaded.as_ref().ok(),
            parent.turn_model_source.clone(),
            parent.model_name.clone(),
            false,
            None,
            self.definition.temperature,
        );
        let max_output_tokens = self
            .definition
            .max_turn_output_tokens
            .unwrap_or(crate::openhuman::inference::provider::AGENT_TURN_MAX_OUTPUT_TOKENS);
        let system_prompt = self.build_tinyagents_system_prompt(&parent, &model)?;

        let mut policy = RunPolicy::default();
        policy.limits.max_model_calls = self.definition.max_iterations;
        policy.limits.max_tool_calls = self.definition.max_iterations.saturating_mul(8).max(8);
        policy.retry.max_attempts = 1;
        policy.unknown_tool = UnknownToolPolicy::ReturnToolError;
        policy.invalid_args = InvalidArgsPolicy::ReturnToolError;

        let mut harness: AgentHarness<()> = AgentHarness::new();
        harness.with_policy(policy);
        let provider_model = super::model::MaxTokensModel::new(
            source.build_summarizer(&model, self.definition.temperature)?,
            max_output_tokens,
        );
        harness
            .register_model(&model, Arc::new(provider_model))
            .set_default_model(&model);

        let child = SubAgent::new(
            self.definition.id.clone(),
            self.definition.when_to_use.clone(),
            Arc::new(harness),
        )
        .with_system_prompt(system_prompt);
        // Run the summarizer UNARY (non-streaming), not via `invoke_in_parent`.
        //
        // `invoke_in_parent` inherits `parent.streaming`, which is `true` for a
        // chat turn, so the child runs the streaming loop and its per-token
        // deltas land on the shared `EventSink` as parent `AgentProgress::
        // TextDelta`. The web bridge buffers those into `pending_narration`
        // and `flush_interim_narration` publishes them as a `chat_interim`
        // bubble, which is persisted as an `isInterim` agent message — so the
        // internal "[Tool output summary — <tool>]" text was rendered to the
        // user as if it were part of the answer. That defeats the point of the
        // summarizer: it exists to compress a payload for the ORCHESTRATOR'S
        // CONTEXT, and its only consumer here is `run.text()` below.
        //
        // `invoke_with_events` runs the child through the unary path
        // (`run_child(.., streaming = false)`), which per its own contract
        // "leav[es] the parent's event stream unchanged", while still sharing
        // the sink so the sub-agent lifecycle events (started/completed) keep
        // reaching observers. Mirrors `reprompt_for_required_block`, which is
        // likewise deliberately silent about an internal repair call.
        //
        // Two bits of config that `invoke_in_parent` threaded are dropped by
        // this entry point and neither matters here: the child `thread_id` (only
        // used to attribute events we no longer stream) and the inherited
        // `max_turn_output_tokens` (already enforced independently by the
        // `MaxTokensModel` wrapper above).
        let run = child
            .invoke_with_events(&(), (), parent_ctx.depth(), prompt, &parent_ctx.events)
            .await?;
        Ok(run.text().unwrap_or_default())
    }

    fn build_tinyagents_system_prompt(
        &self,
        parent: &ParentExecutionContext,
        model: &str,
    ) -> Result<String> {
        let prompt_tools = Vec::new();
        let visible_tool_names = HashSet::new();
        let connected_identities_md =
            crate::openhuman::agent::prompts::render_connected_identities();
        let prompt_ctx = crate::openhuman::agent::context::prompt::PromptContext {
            workspace_dir: &parent.workspace_dir,
            model_name: model,
            agent_id: &self.definition.id,
            tools: &prompt_tools,
            workflows: parent.workflows.as_slice(),
            dispatcher_instructions: "",
            learned: crate::openhuman::agent::context::prompt::LearnedContextData::default(),
            visible_tool_names: &visible_tool_names,
            tool_call_format: parent.tool_call_format,
            connected_integrations: &parent.connected_integrations,
            connected_identities_md,
            include_profile: !self.definition.omit_profile,
            include_memory_md: !self.definition.omit_memory_md,
            curated_snapshot: None,
            user_identity: crate::openhuman::desktop::app_state::peek_cached_current_user_identity(
            ),
            personality_soul_md: None,
            personality_memory_md: None,
            personality_roster: vec![],
            // AGENTS.md layers are intentionally excluded from the payload
            // summarizer. This is a narrow internal utility that condenses an
            // oversized tool payload into a summary — it does no project work in
            // the action directory, so standing project instructions are pure
            // noise here and would waste the tight token budget this summary
            // path is trying to reclaim. Unlike the user-facing agents it also
            // builds its dynamic prompt directly (not via
            // `SystemPromptBuilder::from_dynamic`), so it is deliberately outside
            // the AGENTS.md injection contract that the main + sub-agent prompt
            // paths honour.
            agents_md_global: None,
            agents_md_local: None,
        };

        let system_prompt = match &self.definition.system_prompt {
            PromptSource::Dynamic(build) => build(&prompt_ctx)?,
            PromptSource::Inline(prompt) => prompt.clone(),
            PromptSource::File { path } => {
                return Err(anyhow!(
                    "payload summarizer invoke_in_parent does not support file prompt source: {path}"
                ));
            }
        };
        Ok(subagent_runner::append_subagent_role_contract(
            system_prompt,
            &self.definition.id,
        ))
    }

    fn handle_summarizer_result(
        &self,
        tool_name: &str,
        raw: &str,
        started: std::time::Instant,
        outcome: Result<String>,
    ) -> Result<SummarizeOutcome> {
        match outcome {
            Ok(output) => {
                let summary = output.trim().to_string();
                if summary.is_empty() {
                    warn!(
                        tool = tool_name,
                        "[payload_summarizer] summarizer returned empty response, falling through"
                    );
                    self.record_failure();
                    return Ok(SummarizeOutcome::Unavailable(UnavailableReason::Failed));
                }
                if summary.len() >= raw.len() {
                    warn!(
                        tool = tool_name,
                        summary_bytes = summary.len(),
                        raw_bytes = raw.len(),
                        "[payload_summarizer] summary not smaller than raw payload, falling through"
                    );
                    self.record_failure();
                    return Ok(SummarizeOutcome::Unavailable(UnavailableReason::Failed));
                }
                self.record_success();
                let summary_bytes = summary.len();
                let original_bytes = raw.len();
                let used_pct = summary_bytes
                    .saturating_mul(100)
                    .checked_div(original_bytes)
                    .unwrap_or(0);
                let reduction_pct = 100usize.saturating_sub(used_pct);
                info!(
                    tool = tool_name,
                    original_bytes = original_bytes,
                    summary_bytes = summary_bytes,
                    reduction_pct = reduction_pct,
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    "[payload_summarizer] compressed successfully"
                );
                Ok(SummarizeOutcome::Summarized(SummarizedPayload {
                    summary,
                    original_bytes,
                    summary_bytes,
                }))
            }
            Err(e) => {
                warn!(
                    tool = tool_name,
                    error = %e,
                    "[payload_summarizer] sub-agent dispatch failed, falling through to raw payload"
                );
                self.record_failure();
                Ok(SummarizeOutcome::Unavailable(UnavailableReason::Failed))
            }
        }
    }
}

/// Rough token estimate: ~4 characters per token. Mirrors
/// [`crate::openhuman::memory::tree::tree_runtime::estimate_tokens`] but
/// returns `usize` (not `u32`) and lives here to keep the tinyagents adapter
/// independent from the tree summarizer.
fn estimate_tokens(text: &str) -> usize {
    text.len().div_ceil(4)
}

/// Build the user-message prompt fed into the summarizer sub-agent.
///
/// Wraps the raw payload in `--- BEGIN ---` / `--- END ---` markers so
/// the sub-agent can unambiguously distinguish the payload boundary
/// from other prompt scaffolding. The tool name and optional parent
/// task hint are surfaced before the payload so the summarizer can
/// prioritize facts relevant to the parent's intent.
fn build_summarizer_prompt(tool_name: &str, parent_task_hint: Option<&str>, raw: &str) -> String {
    let hint_line = parent_task_hint
        .map(|h| format!("Parent task hint: {}\n\n", h))
        .unwrap_or_default();
    format!(
        "Tool name: {}\n\n{}Raw tool output (summarize per the extraction contract in your system prompt):\n\n--- BEGIN ---\n{}\n--- END ---",
        tool_name, hint_line, raw
    )
}

#[cfg(test)]
#[path = "payload_summarizer_tests.rs"]
mod tests;
