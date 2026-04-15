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
//! [`PayloadSummarizer::maybe_summarize`] returns `Ok(None)` (i.e.
//! pass-through, do nothing) when:
//!
//! * The raw payload is below
//!   [`SubagentPayloadSummarizer::threshold_bytes`] (default 100 KB —
//!   small payloads aren't worth an extra LLM round-trip).
//! * The raw payload is above
//!   [`SubagentPayloadSummarizer::max_payload_bytes`] (default 5 MB —
//!   too big to summarize cost-effectively; existing
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
//! checks `agent_id == "orchestrator"`). Welcome, skills_agent,
//! researcher, planner, archivist, and every other typed sub-agent get
//! `None` and their tool results are untouched. The summarizer itself
//! is also `None` so it can never recursively summarize its own input.

use anyhow::Result;
use async_trait::async_trait;
use std::sync::{Arc, Mutex};
use tracing::{debug, info, warn};

use super::definition::AgentDefinition;
use super::subagent_runner::{self, SubagentRunOptions};

/// Outcome returned by [`PayloadSummarizer::maybe_summarize`].
///
/// `Ok(None)` from `maybe_summarize` means the caller should keep the
/// raw payload unchanged. `Ok(Some(...))` means the caller should
/// replace the raw payload with [`SummarizedPayload::summary`] before
/// appending it to agent history.
#[derive(Debug, Clone)]
pub struct SummarizedPayload {
    /// The compressed summary text. Replaces the raw tool output.
    pub summary: String,
    /// Original payload size in bytes — for logging/observability.
    pub original_bytes: usize,
    /// Compressed summary size in bytes — for logging/observability.
    pub summary_bytes: usize,
}

/// Trait for anything that can compress a tool result before it enters
/// agent history. Implementations decide the threshold, the dispatch
/// mechanism, and the failure policy.
///
/// Wired into the tool-execution sites in
/// [`super::tool_loop::run_tool_call_loop`] and
/// [`crate::openhuman::agent::harness::session::Agent::execute_tool_call`]
/// via an `Option<&dyn PayloadSummarizer>` parameter so legacy callers
/// (CLI, REPL, tests, non-orchestrator sub-agents) can pass `None` and
/// keep the existing pass-through behaviour.
#[async_trait]
pub trait PayloadSummarizer: Send + Sync {
    /// Inspect a tool result and decide whether to compress it.
    ///
    /// Returns `Ok(None)` if the payload should be kept as-is, or
    /// `Ok(Some(...))` if the caller should swap it for the
    /// compressed [`SummarizedPayload::summary`].
    ///
    /// Errors are intentionally swallowed by the default implementation
    /// — a failed summarization should never break a tool call. The
    /// trait still returns `Result` so future implementations can
    /// surface fatal misconfigurations.
    async fn maybe_summarize(
        &self,
        tool_name: &str,
        parent_task_hint: Option<&str>,
        raw: &str,
    ) -> Result<Option<SummarizedPayload>>;
}

/// Default implementation that dispatches the `summarizer` sub-agent
/// via [`subagent_runner::run_subagent`].
///
/// Holds the `summarizer` agent definition (resolved once at agent
/// build time from the global
/// [`super::definition::AgentDefinitionRegistry`]) plus the threshold
/// knobs and a small failure counter that acts as a session-scoped
/// circuit breaker.
pub struct SubagentPayloadSummarizer {
    /// The `summarizer` agent definition. Cloned from the registry at
    /// agent build time so the runner doesn't have to re-resolve it
    /// per call.
    definition: AgentDefinition,
    /// Lower bound: tool results smaller than this are passed through
    /// untouched. Default is `summarizer_payload_threshold_bytes` from
    /// [`crate::openhuman::config::ContextConfig`] (100 KB).
    threshold_bytes: usize,
    /// Upper bound: tool results larger than this are also passed
    /// through (no LLM call) and fall through to the existing
    /// `tool_result_budget_bytes` truncation downstream. Default is
    /// `summarizer_max_payload_bytes` from
    /// [`crate::openhuman::config::ContextConfig`] (5 MB).
    max_payload_bytes: usize,
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
    pub fn new(
        definition: AgentDefinition,
        threshold_bytes: usize,
        max_payload_bytes: usize,
    ) -> Self {
        Self {
            definition,
            threshold_bytes,
            max_payload_bytes,
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
    async fn maybe_summarize(
        &self,
        tool_name: &str,
        parent_task_hint: Option<&str>,
        raw: &str,
    ) -> Result<Option<SummarizedPayload>> {
        // ── 1. Pass-through checks ─────────────────────────────────────
        if raw.len() < self.threshold_bytes {
            debug!(
                tool = tool_name,
                bytes = raw.len(),
                threshold = self.threshold_bytes,
                "[payload_summarizer] below threshold, passing through"
            );
            return Ok(None);
        }
        if raw.len() > self.max_payload_bytes {
            warn!(
                tool = tool_name,
                bytes = raw.len(),
                max = self.max_payload_bytes,
                "[payload_summarizer] payload exceeds max cap, skipping summarization (will be truncated downstream)"
            );
            return Ok(None);
        }
        if self.breaker_tripped() {
            warn!(
                tool = tool_name,
                bytes = raw.len(),
                "[payload_summarizer] circuit breaker tripped, skipping summarization"
            );
            return Ok(None);
        }

        info!(
            tool = tool_name,
            bytes = raw.len(),
            "[payload_summarizer] dispatching summarizer sub-agent"
        );

        // ── 2. Build the sub-agent prompt ─────────────────────────────
        let prompt = build_summarizer_prompt(tool_name, parent_task_hint, raw);

        // ── 3. Dispatch via subagent_runner ───────────────────────────
        let started = std::time::Instant::now();
        let outcome =
            subagent_runner::run_subagent(&self.definition, &prompt, SubagentRunOptions::default())
                .await;

        // ── 4. Handle result ─────────────────────────────────────────
        match outcome {
            Ok(run) => {
                let summary = run.output.trim().to_string();
                if summary.is_empty() {
                    warn!(
                        tool = tool_name,
                        "[payload_summarizer] summarizer returned empty response, falling through"
                    );
                    self.record_failure();
                    return Ok(None);
                }
                if summary.len() >= raw.len() {
                    warn!(
                        tool = tool_name,
                        summary_bytes = summary.len(),
                        raw_bytes = raw.len(),
                        "[payload_summarizer] summary not smaller than raw payload, falling through"
                    );
                    self.record_failure();
                    return Ok(None);
                }
                self.record_success();
                let summary_bytes = summary.len();
                let original_bytes = raw.len();
                let reduction_pct = if original_bytes == 0 {
                    0
                } else {
                    100usize.saturating_sub(summary_bytes.saturating_mul(100) / original_bytes)
                };
                info!(
                    tool = tool_name,
                    original_bytes = original_bytes,
                    summary_bytes = summary_bytes,
                    reduction_pct = reduction_pct,
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    "[payload_summarizer] compressed successfully"
                );
                Ok(Some(SummarizedPayload {
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
                Ok(None)
            }
        }
    }
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
mod tests {
    use super::*;
    use crate::openhuman::agent::harness::definition::{
        AgentDefinition, DefinitionSource, ModelSpec, PromptSource, SandboxMode, ToolScope,
    };

    fn dummy_definition() -> AgentDefinition {
        AgentDefinition {
            id: "summarizer".into(),
            when_to_use: "test".into(),
            display_name: Some("Summarizer".into()),
            system_prompt: PromptSource::Inline("test prompt".into()),
            omit_identity: true,
            omit_memory_context: true,
            omit_safety_preamble: true,
            omit_skills_catalog: true,
            omit_profile: true,
            omit_memory_md: true,
            model: ModelSpec::Hint("summarization".into()),
            temperature: 0.2,
            tools: ToolScope::Named(vec![]),
            disallowed_tools: vec![],
            skill_filter: None,
            category_filter: None,
            max_iterations: 1,
            timeout_secs: None,
            sandbox_mode: SandboxMode::None,
            background: false,
            uses_fork_context: false,
            subagents: vec![],
            delegate_name: None,
            source: DefinitionSource::Builtin,
        }
    }

    #[tokio::test]
    async fn maybe_summarize_returns_none_below_threshold() {
        let summarizer = SubagentPayloadSummarizer::new(dummy_definition(), 100_000, 5_242_880);
        let raw = "x".repeat(1_024); // 1 KB
        let outcome = summarizer
            .maybe_summarize("test_tool", None, &raw)
            .await
            .expect("below-threshold check should not error");
        assert!(
            outcome.is_none(),
            "1 KB payload below 100 KB threshold should be passed through"
        );
    }

    #[tokio::test]
    async fn maybe_summarize_returns_none_above_max_cap() {
        let summarizer = SubagentPayloadSummarizer::new(dummy_definition(), 100_000, 5_242_880);
        // 6 MB payload — above the 5 MB hard cap.
        let raw = "x".repeat(6 * 1024 * 1024);
        let outcome = summarizer
            .maybe_summarize("test_tool", None, &raw)
            .await
            .expect("above-cap check should not error");
        assert!(
            outcome.is_none(),
            "6 MB payload above 5 MB cap should be passed through (truncation handles it downstream)"
        );
    }

    #[tokio::test]
    async fn maybe_summarize_returns_none_when_breaker_tripped() {
        let summarizer = SubagentPayloadSummarizer::new(dummy_definition(), 100_000, 5_242_880);
        // Manually trip the breaker by recording 3 failures.
        summarizer.record_failure();
        summarizer.record_failure();
        summarizer.record_failure();
        assert!(summarizer.breaker_tripped(), "breaker should be tripped");

        // 200 KB payload — would normally be summarized, but breaker is tripped.
        let raw = "x".repeat(200_000);
        let outcome = summarizer
            .maybe_summarize("test_tool", None, &raw)
            .await
            .expect("breaker check should not error");
        assert!(
            outcome.is_none(),
            "tripped breaker must short-circuit before any sub-agent dispatch"
        );
    }

    #[test]
    fn build_summarizer_prompt_includes_tool_name_and_hint() {
        let prompt = build_summarizer_prompt(
            "GITHUB_LIST_ISSUES",
            Some("find the most urgent open issues"),
            "{\"issues\": [{\"id\": 1}]}",
        );
        assert!(prompt.contains("GITHUB_LIST_ISSUES"));
        assert!(prompt.contains("find the most urgent open issues"));
        assert!(prompt.contains("Parent task hint:"));
        assert!(prompt.contains("--- BEGIN ---"));
        assert!(prompt.contains("--- END ---"));
        assert!(prompt.contains("{\"issues\": [{\"id\": 1}]}"));
    }

    #[test]
    fn build_summarizer_prompt_omits_hint_when_none() {
        let prompt = build_summarizer_prompt("file_read", None, "log line 1\nlog line 2");
        assert!(prompt.contains("file_read"));
        assert!(prompt.contains("--- BEGIN ---"));
        assert!(prompt.contains("--- END ---"));
        assert!(prompt.contains("log line 1"));
        assert!(
            !prompt.contains("Parent task hint:"),
            "no hint line should be present when hint is None"
        );
    }

    #[test]
    fn record_success_resets_breaker() {
        let summarizer = SubagentPayloadSummarizer::new(dummy_definition(), 100_000, 5_242_880);
        summarizer.record_failure();
        summarizer.record_failure();
        assert!(!summarizer.breaker_tripped());
        summarizer.record_success();
        // Even one more failure now should not trip — counter was reset.
        summarizer.record_failure();
        assert!(!summarizer.breaker_tripped());
    }
}
