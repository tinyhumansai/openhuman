use crate::openhuman::agent::progress::AgentProgress;
use crate::openhuman::inference::provider::{ChatMessage, Provider};
use crate::openhuman::tools::policy::{DefaultToolPolicy, ToolPolicy};
use crate::openhuman::tools::Tool;
use anyhow::Result;
use std::collections::HashSet;

use super::payload_summarizer::PayloadSummarizer;

/// Minimum characters per chunk when relaying LLM text to a streaming draft.
pub(crate) const STREAM_CHUNK_MIN_CHARS: usize = 80;

/// Default maximum agentic tool-use iterations per user message to prevent runaway loops.
/// Used as a safe fallback when `max_tool_iterations` is unset or configured as zero.
pub(crate) const DEFAULT_MAX_TOOL_ITERATIONS: usize = 10;

/// Repeated-failure circuit breaker. The plain iteration cap lets an agent grind
/// the same dead-end (e.g. re-running `pip install` when there is no pip) until
/// `max_iterations`, then return an opaque `MaxIterationsExceeded` that the caller
/// just re-spawns — losing the failure context. These thresholds let the loop bail
/// EARLY with a root-cause summary instead.
///
/// If the SAME `(tool, args)` call fails this many times, the agent is repeating a
/// known-failed action verbatim — stop.
pub(crate) const REPEAT_FAILURE_THRESHOLD: u32 = 3;
/// If this many tool calls fail back-to-back with no success in between (even with
/// varied args), the agent is making no progress — stop.
pub(crate) const NO_PROGRESS_FAILURE_THRESHOLD: u32 = 6;
/// Hard policy rejections (a security block or a gate denial) are deterministic:
/// the identical `(tool, args)` call provably cannot succeed. Halt on the FIRST
/// verbatim repeat — i.e. the second identical attempt — rather than letting the
/// agent burn the generic [`REPEAT_FAILURE_THRESHOLD`] on a doomed call. The first
/// occurrence is allowed through so the model can read the "do not retry" reason
/// and pivot to a different, allowed approach.
pub(crate) const HARD_REJECT_REPEAT_THRESHOLD: u32 = 2;

/// Classification of a deterministic, recognizable policy rejection, detected via
/// the stable markers the security/approval layers emit
/// ([`crate::openhuman::security::POLICY_BLOCKED_MARKER`] /
/// [`POLICY_DENIED_MARKER`](crate::openhuman::security::POLICY_DENIED_MARKER)).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum HardReject {
    /// Permanent for this tier (read-only write, forbidden/credential path,
    /// disallowed command) — never succeeds on retry.
    Blocked,
    /// User denied / approval timed out this turn — re-asking the identical call
    /// only re-prompts.
    Denied,
}

/// Recognize a hard policy rejection from a tool result. Matches anywhere in the
/// string (not just the prefix) so it survives the `Error: …` wrapping the tool
/// layer adds. `Blocked` takes precedence over `Denied` if both somehow appear.
pub(crate) fn hard_reject_kind(result: &str) -> Option<HardReject> {
    if result.contains(crate::openhuman::security::POLICY_BLOCKED_MARKER) {
        Some(HardReject::Blocked)
    } else if result.contains(crate::openhuman::security::POLICY_DENIED_MARKER) {
        Some(HardReject::Denied)
    } else {
        None
    }
}

/// Shared repeated-failure circuit breaker, used by BOTH agent loops
/// (`run_tool_call_loop` here and `run_inner_loop` in `subagent_runner`) so they
/// can't drift. Tracks per-`(tool,args)`-signature failure counts and a
/// consecutive-failure run within a single agent turn; [`Self::record`] returns
/// a root-cause halt summary once a threshold trips.
#[derive(Default)]
pub(crate) struct RepeatFailureGuard {
    sig_counts: std::collections::HashMap<String, u32>,
    consecutive: u32,
}

impl RepeatFailureGuard {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Record one tool-call outcome. `args_sig` is a stable string form of the
    /// arguments (e.g. the command). Returns `Some(summary)` when the breaker
    /// trips — the caller should stop the loop and return that summary as the
    /// agent's result instead of grinding to `max_iterations`.
    pub(crate) fn record(
        &mut self,
        tool: &str,
        args_sig: &str,
        success: bool,
        result: &str,
    ) -> Option<String> {
        if success {
            self.consecutive = 0;
            return None;
        }
        self.consecutive += 1;
        let count = {
            let c = self
                .sig_counts
                .entry(format!("{tool}|{args_sig}"))
                .or_insert(0);
            *c += 1;
            *c
        };
        // Hard policy rejections trip on the first verbatim repeat; everything
        // else uses the generic identical-retry threshold.
        let hard = hard_reject_kind(result);
        let repeat_threshold = if hard.is_some() {
            HARD_REJECT_REPEAT_THRESHOLD
        } else {
            REPEAT_FAILURE_THRESHOLD
        };
        if count >= repeat_threshold {
            return Some(match hard {
                Some(HardReject::Blocked) => format!(
                    "Stopping: the `{tool}` call is blocked by the security policy and was \
                     re-issued with identical arguments — it can never succeed this way. \
                     Reason:\n{}\n\nDo not repeat this call; use an allowed alternative or report \
                     that it can't be done here.",
                    truncate_for_halt(result),
                ),
                Some(HardReject::Denied) => format!(
                    "Stopping: the `{tool}` call was denied and re-issued unchanged — re-asking \
                     will not change the answer. Reason:\n{}\n\nDo not repeat this call; take a \
                     different approach or report that it can't be done here.",
                    truncate_for_halt(result),
                ),
                None => format!(
                    "Stopping: the `{tool}` call was retried {count} times with identical \
                     arguments and kept failing — repeating it will not help. Last error:\n{}\n\n\
                     This looks unrecoverable in the current environment (e.g. a missing \
                     tool/dependency that cannot be installed here). Report this back instead of \
                     retrying.",
                    truncate_for_halt(result),
                ),
            });
        }
        if self.consecutive >= NO_PROGRESS_FAILURE_THRESHOLD {
            return Some(format!(
                "Stopping: {} tool calls in a row failed with no progress. Last error (from \
                 `{tool}`):\n{}\n\nDifferent commands are all failing — the goal looks unreachable \
                 in this environment. Report this back instead of retrying.",
                self.consecutive,
                truncate_for_halt(result),
            ));
        }
        None
    }
}

/// Clamp the last-error text embedded in a circuit-breaker halt summary so a huge
/// tool error (already capped at 1MB upstream) can't blow up the agent's result.
pub(crate) fn truncate_for_halt(s: &str) -> String {
    const MAX: usize = 600;
    if s.chars().count() <= MAX {
        return s.to_string();
    }
    let head: String = s.chars().take(MAX).collect();
    format!("{head}\n… [truncated]")
}

/// Execute a single turn of the agent loop: send messages, parse tool calls,
/// execute tools, and loop until the LLM produces a final text response.
/// When `silent` is true, suppresses stdout (for channel use).
///
/// This is a thin wrapper around [`run_tool_call_loop`] with the per-agent
/// filter and extra-tool plumbing disabled — i.e. the LLM sees the entire
/// `tools_registry` unchanged. Used by legacy call sites and harness tests
/// that don't need agent-aware scoping.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn agent_turn(
    provider: &dyn Provider,
    history: &mut Vec<ChatMessage>,
    tools_registry: &[Box<dyn Tool>],
    provider_name: &str,
    model: &str,
    temperature: f64,
    silent: bool,
    multimodal_config: &crate::openhuman::config::MultimodalConfig,
    max_tool_iterations: usize,
    payload_summarizer: Option<&dyn PayloadSummarizer>,
) -> Result<String> {
    let default_policy = DefaultToolPolicy;
    run_tool_call_loop(
        provider,
        history,
        tools_registry,
        provider_name,
        model,
        temperature,
        silent,
        "channel",
        multimodal_config,
        max_tool_iterations,
        None,
        None,
        &[],
        None,
        payload_summarizer,
        &default_policy,
    )
    .await
}

/// Execute a single turn of the agent loop: send messages, parse tool calls,
/// execute tools, and loop until the LLM produces a final text response.
///
/// # Per-agent tool scoping
///
/// The last two parameters support per-agent tool filtering without
/// requiring callers to build a filtered copy of the (non-`Clone`able)
/// tool registry:
///
/// * `visible_tool_names` — optional whitelist of tool names that are
///   allowed to reach the LLM. When `Some(set)`, only tools whose
///   `name()` is present in the set contribute to the function-calling
///   schema and are eligible for execution; every other tool in the
///   registry is hidden from the model and rejected if the model
///   somehow emits a call for it. When `None`, no filtering is applied
///   and every tool in the combined registry is visible (the legacy
///   behaviour used by CLI/REPL and harness tests).
///
/// * `extra_tools` — per-turn synthesised tools to splice alongside the
///   persistent `tools_registry`. The agent-dispatch path uses this to
///   surface delegation tools (`research`, `plan`,
///   `delegate_to_integrations_agent`, …) that are synthesised fresh
///   per turn from the active agent's `subagents` field and the
///   current Composio integration list, and therefore are not
///   registered in the global startup-time registry.
///
/// The combined tool list seen by the LLM this turn is
/// `tools_registry.iter().chain(extra_tools.iter())`, further narrowed
/// by `visible_tool_names` when supplied.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_tool_call_loop(
    provider: &dyn Provider,
    history: &mut Vec<ChatMessage>,
    tools_registry: &[Box<dyn Tool>],
    provider_name: &str,
    model: &str,
    temperature: f64,
    silent: bool,
    // Retained in the harness signature (callers pass their channel) but no
    // longer consumed here since the legacy CLI approval prompt was removed —
    // approval now flows through the process-global `ApprovalGate`.
    _channel_name: &str,
    multimodal_config: &crate::openhuman::config::MultimodalConfig,
    max_tool_iterations: usize,
    on_delta: Option<tokio::sync::mpsc::Sender<String>>,
    visible_tool_names: Option<&HashSet<String>>,
    extra_tools: &[Box<dyn Tool>],
    on_progress: Option<tokio::sync::mpsc::Sender<AgentProgress>>,
    payload_summarizer: Option<&dyn PayloadSummarizer>,
    tool_policy: &dyn ToolPolicy,
) -> Result<String> {
    let max_iterations = if max_tool_iterations == 0 {
        DEFAULT_MAX_TOOL_ITERATIONS
    } else {
        max_tool_iterations
    };

    // The agentic loop itself now lives in the shared turn engine; this
    // function is a thin adapter that builds the channel/CLI tool source
    // (registry + per-turn extras, visibility whitelist, pluggable policy)
    // and hands off. The signature is retained verbatim so existing callers
    // (the `agent.run_turn` bus handler, triage, the payload summarizer, and
    // the harness test suite) are unaffected.
    log::debug!(
        "[tool-loop] Registry has {} tool(s), extra {} tool(s), filter={}",
        tools_registry.len(),
        extra_tools.len(),
        visible_tool_names
            .map(|s| format!("whitelist({})", s.len()))
            .unwrap_or_else(|| "none".to_string()),
    );
    let mut tool_source = super::engine::RegistryToolSource::new(
        tools_registry,
        extra_tools,
        visible_tool_names,
        tool_policy,
        payload_summarizer,
    );
    let progress = super::engine::TurnProgress::new(on_progress);
    let mut observer = super::engine::NullObserver;
    let checkpoint = super::engine::ErrorCheckpoint;
    let parser = super::engine::DefaultParser;
    super::engine::run_turn_engine(
        provider,
        history,
        &mut tool_source,
        &progress,
        &mut observer,
        &checkpoint,
        &parser,
        provider_name,
        model,
        temperature,
        silent,
        multimodal_config,
        max_iterations,
        on_delta,
    )
    .await
    .map(|outcome| outcome.text)
}

#[cfg(test)]
#[path = "tool_loop_tests.rs"]
mod tests;
