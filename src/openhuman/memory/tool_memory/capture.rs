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
//! [`ToolTrackerHook`]: crate::openhuman::learning::ToolTrackerHook
//! [`ToolMemoryStore`]: super::store::ToolMemoryStore

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;

use super::store::ToolMemoryStore;
use super::types::{ToolMemoryPriority, ToolMemorySource};
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
            store: ToolMemoryStore::new(memory),
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
        if !(lower.contains("never ") || lower.contains("don't ") || lower.contains("do not "))
            && !stop_imperative
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
        for raw_line in trimmed.split(|c: char| matches!(c, '.' | '\n' | ';')) {
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
mod tests {
    use super::*;
    use crate::openhuman::agent::hooks::ToolCallRecord;
    use crate::openhuman::memory::tool_memory::store::ToolMemoryStore;
    use crate::openhuman::memory::tool_memory::test_helpers::MockMemory;

    fn ctx_with(message: &str, tool_calls: Vec<ToolCallRecord>) -> TurnContext {
        TurnContext {
            user_message: message.into(),
            assistant_response: "ok".into(),
            tool_calls,
            turn_duration_ms: 1,
            session_id: None,
            iteration_count: 1,
        }
    }

    fn call(name: &str, success: bool) -> ToolCallRecord {
        ToolCallRecord {
            name: name.into(),
            arguments: serde_json::json!({}),
            success,
            output_summary: if success {
                "ok".into()
            } else {
                "permission denied".into()
            },
            duration_ms: 10,
        }
    }

    #[test]
    fn extract_user_edicts_picks_up_never_phrase() {
        let edicts = ToolMemoryCaptureHook::extract_user_edicts(
            "Never email Sarah at sarah@example.com — she does not want updates.",
            &[call("send_email", true)],
        );
        assert!(!edicts.is_empty(), "expected at least one captured edict");
        let (tool, body) = &edicts[0];
        assert_eq!(
            tool, "send_email",
            "should map 'email' alias to send_email tool"
        );
        assert!(body.to_lowercase().contains("never email"));
    }

    #[test]
    fn extract_user_edicts_handles_dont_and_stop_phrases() {
        let edicts = ToolMemoryCaptureHook::extract_user_edicts(
            "Don't run shell commands with sudo. Stop using browser for that.",
            &[call("shell", true), call("browser", true)],
        );
        assert_eq!(edicts.len(), 2, "should capture each imperative separately");
    }

    #[test]
    fn extract_user_edicts_returns_empty_when_no_edict_present() {
        let edicts = ToolMemoryCaptureHook::extract_user_edicts(
            "Send Sarah an update when you can.",
            &[call("send_email", true)],
        );
        assert!(edicts.is_empty());
    }

    #[test]
    fn extract_user_edicts_falls_back_to_first_tool_when_no_alias_match() {
        let edicts = ToolMemoryCaptureHook::extract_user_edicts(
            "Never do that automatically.",
            &[call("calendar", true)],
        );
        assert_eq!(edicts.len(), 1);
        assert_eq!(edicts[0].0, "calendar");
    }

    #[test]
    fn extract_user_edicts_uses_sentinel_when_no_tools_ran() {
        let edicts = ToolMemoryCaptureHook::extract_user_edicts("Never do that.", &[]);
        assert_eq!(edicts.len(), 1);
        assert_eq!(edicts[0].0, "__unscoped__");
    }

    #[test]
    fn extract_repeated_failures_needs_two_or_more_failures() {
        let observations = ToolMemoryCaptureHook::extract_repeated_failures(&[
            call("shell", false),
            call("shell", false),
            call("shell", true),
        ]);
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].0, "shell");
        assert!(observations[0].1.contains("failed 2 times"));
    }

    #[test]
    fn extract_repeated_failures_ignores_single_failures() {
        let observations =
            ToolMemoryCaptureHook::extract_repeated_failures(&[call("shell", false)]);
        assert!(observations.is_empty());
    }

    #[tokio::test]
    async fn on_turn_complete_persists_critical_rule_for_user_edict() {
        let memory: Arc<dyn Memory> = Arc::new(MockMemory::default());
        let store = ToolMemoryStore::new(memory.clone());
        let hook = ToolMemoryCaptureHook::from_store(store.clone(), true);

        hook.on_turn_complete(&ctx_with(
            "Never email Sarah — she opted out.",
            vec![call("send_email", true)],
        ))
        .await
        .unwrap();

        let rules = store.list_rules("send_email").await.unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].priority, ToolMemoryPriority::Critical);
        assert_eq!(rules[0].source, ToolMemorySource::UserExplicit);
        assert!(rules[0].tags.contains(&"user-edict".to_string()));
    }

    #[tokio::test]
    async fn on_turn_complete_no_op_when_disabled() {
        let memory: Arc<dyn Memory> = Arc::new(MockMemory::default());
        let store = ToolMemoryStore::new(memory.clone());
        let hook = ToolMemoryCaptureHook::from_store(store.clone(), false);
        hook.on_turn_complete(&ctx_with(
            "Never email Sarah.",
            vec![call("send_email", true)],
        ))
        .await
        .unwrap();
        assert!(store.list_rules("send_email").await.unwrap().is_empty());
    }

    /// Safety case (AC #5): "never email Sarah" flows end-to-end from
    /// a user utterance → captured as a Critical rule → surfaces in
    /// the prompt-injection block.
    #[tokio::test]
    async fn safety_case_never_email_sarah_pins_into_prompt_block() {
        let memory: Arc<dyn Memory> = Arc::new(MockMemory::default());
        let store = ToolMemoryStore::new(memory.clone());
        let hook = ToolMemoryCaptureHook::from_store(store.clone(), true);

        // 1. Capture the edict from a normal user turn.
        hook.on_turn_complete(&ctx_with(
            "Never email Sarah at sarah@example.com.",
            vec![call("send_email", true)],
        ))
        .await
        .unwrap();

        // 2. The rule lands in the tool-scoped namespace with Critical
        //    priority — distinct from `tool_effectiveness` / global.
        let stored = store.list_rules("send_email").await.unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].priority, ToolMemoryPriority::Critical);

        // 3. `rules_for_prompt` pulls it eagerly so the session builder
        //    can pin it into the (compression-resistant) system prompt.
        let prompt = store
            .rules_for_prompt(&["send_email".to_string()])
            .await
            .unwrap();
        assert!(prompt.contains_key("send_email"));

        // 4. The rendered block is non-empty and mentions the edict
        //    verbatim — the exact bytes the safety pipeline puts in
        //    front of the agent on every subsequent turn.
        let mut flat: Vec<_> = prompt.into_values().flatten().collect();
        flat.sort_by(|a, b| b.priority.cmp(&a.priority));
        let rendered = crate::openhuman::memory::tool_memory::render_tool_memory_rules(&flat);
        assert!(rendered.contains("Never email Sarah"));
        assert!(rendered.contains("**[critical]**"));
    }

    #[tokio::test]
    async fn on_turn_complete_records_repeated_failure_observation() {
        let memory: Arc<dyn Memory> = Arc::new(MockMemory::default());
        let store = ToolMemoryStore::new(memory.clone());
        let hook = ToolMemoryCaptureHook::from_store(store.clone(), true);
        hook.on_turn_complete(&ctx_with(
            "Try again",
            vec![call("shell", false), call("shell", false)],
        ))
        .await
        .unwrap();
        let rules = store.list_rules("shell").await.unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].priority, ToolMemoryPriority::Normal);
        assert_eq!(rules[0].source, ToolMemorySource::PostTurn);
        assert!(rules[0].tags.contains(&"repeated-failure".to_string()));
    }
}
