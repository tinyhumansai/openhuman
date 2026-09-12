//! Post-turn capture hook for tool-scoped memory.
//!
//! This hook complements the statistics-only [`ToolTrackerHook`] —
//! `tool_effectiveness` records *what happened* (counts, error patterns),
//! while [`ToolMemoryCaptureHook`] records *what to do about it* as
//! actionable [`ToolMemoryRule`]s in the tool-scoped namespace.
//!
//! Two capture paths fire automatically after every turn:
//!
//! 1. **User edicts** — phrases like `never <verb> <object>`,
//!    `don't <verb> …`, or `stop <verb>ing …` in the user message are
//!    promoted to a `Critical` rule attached to the matching tool when
//!    one of the turn's tool calls plausibly applies. This covers the
//!    "never email Sarah" safety case from the spec.
//!
//! 2. **Repeated tool failures** — when a tool fails twice or more
//!    within a single turn, a `Normal`-priority observation is captured
//!    so the agent has a record next time it considers that tool.
//!
//! Both paths are conservative — they only fire on clear signals, and
//! the captured rule body always points back to the user's own words so
//! a reviewer can see exactly what triggered it.
//!
//! Captured rules are stored via [`ToolMemoryStore`] in the
//! `tool-{tool_name}` namespace, never in `global` or
//! `tool_effectiveness`.
//!
//! [`ToolTrackerHook`]: crate::openhuman::agent::learning::ToolTrackerHook

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;

use super::{tool_memory_store, ToolMemoryPriority, ToolMemorySource, ToolMemoryStore};
use crate::openhuman::agent::hooks::{PostTurnHook, ToolCallRecord, TurnContext};
use crate::openhuman::memory::Memory;

/// Maximum length (chars) of the captured rule body — keeps malformed or
/// runaway input from bloating the namespace.
const MAX_RULE_LEN: usize = 240;

/// Post-turn hook that captures durable tool-scoped rules.
pub struct ToolMemoryCaptureHook {
    store: ToolMemoryStore,
    enabled: bool,
}

impl ToolMemoryCaptureHook {
    /// Build a new capture hook backed by the given memory.
    pub fn new(memory: Arc<dyn Memory>, enabled: bool) -> Self {
        Self {
            store: tool_memory_store(memory),
            enabled,
        }
    }

    /// Build a hook directly over a [`ToolMemoryStore`] — useful for
    /// tests and call sites that already hold a store.
    pub fn from_store(store: ToolMemoryStore, enabled: bool) -> Self {
        Self { store, enabled }
    }

    /// Look at the user message and return any `Critical`-priority rule
    /// patterns it contains, paired with the tool name they apply to.
    ///
    /// Pure / synchronous so it can be unit-tested without a memory
    /// backend.
    pub fn extract_user_edicts(
        user_message: &str,
        tool_calls: &[ToolCallRecord],
    ) -> Vec<(String, String)> {
        let trimmed = user_message.trim();
        if trimmed.is_empty() {
            return Vec::new();
        }
        let lower = trimmed.to_lowercase();
        // Only treat "stop" as an imperative edict when it appears at a
        // sentence boundary (start of message or after ". "/"\n"), so routine
        // phrases like "I want to stop working" don't trigger false captures.
        let stop_imperative =
            lower.starts_with("stop ") || lower.contains(". stop ") || lower.contains("\nstop ");
        if !(lower.contains("never ")
            || lower.contains("don't ")
            || lower.contains("do not ")
            || stop_imperative)
        {
            return Vec::new();
        }

        // Default tool: the first tool that ran in the turn. When there
        // were no tool calls we still want to capture user edicts so
        // they survive into the next turn — those land under the
        // `__unscoped__` tool name and the agent can refile them.
        let default_tool = tool_calls
            .first()
            .map(|tc| tc.name.clone())
            .unwrap_or_else(|| "__unscoped__".to_string());

        let mut out = Vec::new();
        for raw_line in trimmed.split(['.', '\n', ';']) {
            let line = raw_line.trim();
            if line.is_empty() {
                continue;
            }
            let lower_line = line.to_lowercase();
            let is_edict = lower_line.starts_with("never ")
                || lower_line.starts_with("don't ")
                || lower_line.starts_with("do not ")
                || lower_line.starts_with("stop ")
                || lower_line.contains(" never ")
                || lower_line.contains(" don't ")
                || lower_line.contains(" do not ");
            if !is_edict {
                continue;
            }
            let body: String = line.chars().take(MAX_RULE_LEN).collect();
            if body.is_empty() {
                continue;
            }
            let tool =
                pick_tool_for_edict(&body, tool_calls).unwrap_or_else(|| default_tool.clone());
            out.push((tool, body));
        }
        out
    }

    /// Look at the tool-call records and return any (tool_name, body)
    /// pairs that describe repeated failures worth pinning as a
    /// `Normal`-priority observation.
    ///
    /// A tool counts when it failed two or more times in the turn —
    /// transient one-off failures are ignored to keep the namespace
    /// from filling with noise.
    pub fn extract_repeated_failures(tool_calls: &[ToolCallRecord]) -> Vec<(String, String)> {
        let mut tallies: HashMap<&str, (usize, Option<&str>)> = HashMap::new();
        for tc in tool_calls {
            if tc.success {
                continue;
            }
            let entry = tallies.entry(tc.name.as_str()).or_insert((0, None));
            entry.0 += 1;
            if entry.1.is_none() {
                entry.1 = Some(tc.output_summary.as_str());
            }
        }

        let mut out = Vec::new();
        for (tool, (count, sample)) in tallies {
            if count < 2 {
                continue;
            }
            let body = match sample {
                Some(sample) => format!(
                    "Tool failed {count} times in one turn ({sample}). Consider an alternative \
                    approach before retrying."
                ),
                None => format!(
                    "Tool failed {count} times in one turn. Consider an alternative approach \
                    before retrying."
                ),
            };
            out.push((tool.to_string(), body.chars().take(MAX_RULE_LEN).collect()));
        }
        out
    }
}

#[async_trait]
impl PostTurnHook for ToolMemoryCaptureHook {
    fn name(&self) -> &str {
        "tool_memory_capture"
    }

    async fn on_turn_complete(&self, ctx: &TurnContext) -> anyhow::Result<()> {
        if !self.enabled {
            return Ok(());
        }

        for (tool, body) in Self::extract_user_edicts(&ctx.user_message, &ctx.tool_calls) {
            log::debug!(
                "[tool-memory] capturing user edict tool={tool} body_len={}",
                body.len()
            );
            if let Err(err) = self
                .store
                .record(
                    &tool,
                    &body,
                    ToolMemoryPriority::Critical,
                    ToolMemorySource::UserExplicit,
                    vec!["user-edict".into()],
                )
                .await
            {
                log::warn!("[tool-memory] failed to capture user edict for {tool}: {err}");
            }
        }

        for (tool, body) in Self::extract_repeated_failures(&ctx.tool_calls) {
            log::debug!(
                "[tool-memory] capturing repeated failure tool={tool} body_len={}",
                body.len()
            );
            if let Err(err) = self
                .store
                .record(
                    &tool,
                    &body,
                    ToolMemoryPriority::Normal,
                    ToolMemorySource::PostTurn,
                    vec!["repeated-failure".into()],
                )
                .await
            {
                log::warn!(
                    "[tool-memory] failed to capture repeated-failure observation for {tool}: {err}"
                );
            }
        }

        Ok(())
    }
}

/// Helper: emit a [`ToolMemoryRule`] preview without flooding logs with
/// raw user prose.
fn truncate_for_log(body: &str) -> String {
    let mut out: String = body.chars().take(80).collect();
    if body.chars().count() > 80 {
        out.push('…');
    }
    out
}

/// Best-effort match between a user edict and a tool that ran in the
/// turn. We look for the tool name appearing as a word in the edict;
/// when several match, the first call's tool wins.
fn pick_tool_for_edict(body: &str, tool_calls: &[ToolCallRecord]) -> Option<String> {
    if tool_calls.is_empty() {
        return None;
    }
    let lower = body.to_lowercase();
    for tc in tool_calls {
        let needle = tc.name.to_lowercase();
        if needle.is_empty() {
            continue;
        }
        if lower.contains(&needle) {
            return Some(tc.name.clone());
        }
        // Common-noun aliases — match "email" to a tool named
        // "send_email", "gmail_send", etc.
        for alias in tool_aliases(&tc.name) {
            if lower.contains(alias) {
                return Some(tc.name.clone());
            }
        }
    }
    None
}

/// Map a tool name to a small set of common-noun aliases users would
/// say in plain English ("email", "shell", "browser", …). Kept tiny on
/// purpose — anything more ambitious belongs in an LLM extractor.
fn tool_aliases(tool_name: &str) -> Vec<&'static str> {
    let lower = tool_name.to_lowercase();
    let mut out = Vec::new();
    if lower.contains("mail") {
        out.push("email");
        out.push("mail");
    }
    if lower.contains("shell") || lower.contains("bash") || lower.contains("exec") {
        out.push("shell");
        out.push("terminal");
    }
    if lower.contains("browser") || lower.contains("web") || lower.contains("http") {
        out.push("browser");
        out.push("web");
    }
    if lower.contains("slack") {
        out.push("slack");
        out.push("dm");
    }
    out
}

#[cfg(test)]
#[path = "capture_tests.rs"]
mod tests;
