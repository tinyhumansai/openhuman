use async_trait::async_trait;
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use crate::openhuman::agent::hooks::{PostTurnHook, ToolCallRecord, TurnContext};
use crate::openhuman::agent_experience::store::AgentExperienceStore;
use crate::openhuman::agent_experience::types::{
    redact_text, stable_experience_id, AgentExperience, ExperienceOutcome, ExperienceSource,
};
use crate::openhuman::memory::Memory;

const MAX_SUMMARY_CHARS: usize = 280;

pub struct AgentExperienceCaptureHook {
    store: AgentExperienceStore,
    enabled: bool,
}

impl AgentExperienceCaptureHook {
    pub fn new(memory: Arc<dyn Memory>, enabled: bool) -> Self {
        Self {
            store: AgentExperienceStore::new(memory),
            enabled,
        }
    }

    pub fn from_store(store: AgentExperienceStore, enabled: bool) -> Self {
        Self { store, enabled }
    }

    pub fn extract_candidates(ctx: &TurnContext) -> Vec<AgentExperience> {
        let mut candidates = Vec::new();
        if ctx.tool_calls.is_empty() {
            return candidates;
        }

        if let Some(success) = successful_multi_tool_experience(ctx) {
            candidates.push(success);
        }

        candidates.extend(repeated_failure_experiences(ctx));

        if let Some(partial) = partial_success_experience(ctx) {
            let duplicates_existing_candidate = candidates.iter().any(|candidate| {
                candidate.id == partial.id || candidate.outcome == partial.outcome
            });
            if !duplicates_existing_candidate {
                candidates.push(partial);
            }
        }

        candidates
    }
}

#[async_trait]
impl PostTurnHook for AgentExperienceCaptureHook {
    fn name(&self) -> &str {
        "agent_experience_capture"
    }

    async fn on_turn_complete(&self, ctx: &TurnContext) -> anyhow::Result<()> {
        if !self.enabled {
            return Ok(());
        }

        for candidate in Self::extract_candidates(ctx) {
            if let Err(err) = self.store.put(candidate).await {
                log::warn!("[agent-experience] failed to capture turn experience: {err}");
            }
        }
        Ok(())
    }
}

fn successful_multi_tool_experience(ctx: &TurnContext) -> Option<AgentExperience> {
    let successful_calls: Vec<&ToolCallRecord> =
        ctx.tool_calls.iter().filter(|call| call.success).collect();
    let sequence = tool_sequence(&successful_calls);
    if sequence.len() < 2 {
        return None;
    }

    let now = now_ms();
    let summary = truncate_chars(&redact_text(&ctx.user_message), MAX_SUMMARY_CHARS);
    let lesson = format!(
        "For similar tasks, the successful tool sequence was {}.",
        sequence.join(" -> ")
    );
    let reuse_hint = format!(
        "Reuse {} when the task resembles: {summary}",
        sequence.join(" -> ")
    );
    Some(build_experience(
        now,
        ExperienceOutcome::Success,
        None,
        ctx.agent_id.clone(),
        ctx.entrypoint.clone(),
        summary,
        sequence,
        lesson,
        reuse_hint,
        None,
        0.72,
        vec!["tool-loop".into(), "multi-tool-success".into()],
    ))
}

fn repeated_failure_experiences(ctx: &TurnContext) -> Vec<AgentExperience> {
    let mut failures: HashMap<String, Vec<&ToolCallRecord>> = HashMap::new();
    for call in &ctx.tool_calls {
        if !call.success {
            failures.entry(call.name.clone()).or_default().push(call);
        }
    }

    let now = now_ms();
    failures
        .into_iter()
        .filter_map(|(tool, calls)| {
            if calls.len() < 2 {
                return None;
            }
            let error_class = calls
                .first()
                .map(|call| error_class_from_summary(&call.output_summary));
            let summary = truncate_chars(&redact_text(&ctx.user_message), MAX_SUMMARY_CHARS);
            let lesson = format!(
                "{tool} failed {} times in one turn{}.",
                calls.len(),
                error_class
                    .as_deref()
                    .map(|class| format!(" with {class}"))
                    .unwrap_or_default()
            );
            let avoid_hint = format!(
                "Avoid retrying {tool} repeatedly without changing inputs or choosing another tool."
            );
            Some(build_experience(
                now,
                ExperienceOutcome::Failure,
                error_class,
                ctx.agent_id.clone(),
                ctx.entrypoint.clone(),
                summary,
                vec![tool.clone()],
                lesson,
                format!("When {tool} fails repeatedly, inspect the error class before retrying."),
                Some(avoid_hint),
                0.68,
                vec!["tool-loop".into(), "repeated-failure".into()],
            ))
        })
        .collect()
}

fn partial_success_experience(ctx: &TurnContext) -> Option<AgentExperience> {
    let first_failure = ctx.tool_calls.iter().position(|call| !call.success)?;
    let later_success = ctx
        .tool_calls
        .iter()
        .skip(first_failure + 1)
        .any(|call| call.success);
    if !later_success {
        return None;
    }

    let calls: Vec<&ToolCallRecord> = ctx.tool_calls.iter().collect();
    let sequence = tool_sequence(&calls);
    if sequence.len() < 2 {
        return None;
    }

    let now = now_ms();
    let summary = truncate_chars(&redact_text(&ctx.user_message), MAX_SUMMARY_CHARS);
    let lesson = format!(
        "The task recovered after an earlier tool failure by continuing with {}.",
        sequence.join(" -> ")
    );
    Some(build_experience(
        now,
        ExperienceOutcome::Partial,
        None,
        ctx.agent_id.clone(),
        ctx.entrypoint.clone(),
        summary,
        sequence,
        lesson,
        "If the first tool fails, switch strategy instead of repeating the same call.".into(),
        Some("Repeating the same failed call without new evidence delayed progress.".into()),
        0.62,
        vec!["tool-loop".into(), "partial-success".into()],
    ))
}

fn build_experience(
    now: i64,
    outcome: ExperienceOutcome,
    error_class: Option<String>,
    agent_id: Option<String>,
    entrypoint: Option<String>,
    task_summary: String,
    tool_sequence: Vec<String>,
    lesson: String,
    reuse_hint: String,
    avoid_hint: Option<String>,
    confidence: f32,
    tags: Vec<String>,
) -> AgentExperience {
    let tools_used = unique_tools(&tool_sequence);
    let id = stable_experience_id(&task_summary, &tool_sequence, outcome);
    AgentExperience {
        id,
        created_at_ms: now,
        updated_at_ms: now,
        source: ExperienceSource::ToolLoop,
        agent_id: clean_optional(agent_id),
        entrypoint: clean_optional(entrypoint),
        task_fingerprint: stable_task_fingerprint(&task_summary),
        task_summary,
        tools_used,
        tool_sequence,
        outcome,
        error_class,
        lesson: truncate_chars(&redact_text(&lesson), MAX_SUMMARY_CHARS),
        reuse_hint: truncate_chars(&redact_text(&reuse_hint), MAX_SUMMARY_CHARS),
        avoid_hint: avoid_hint.map(|hint| truncate_chars(&redact_text(&hint), MAX_SUMMARY_CHARS)),
        confidence,
        tags,
        payload_hash: None,
        dismissed: false,
    }
}

fn clean_optional(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn tool_sequence(calls: &[&ToolCallRecord]) -> Vec<String> {
    calls
        .iter()
        .map(|call| call.name.trim().to_string())
        .filter(|name| !name.is_empty())
        .collect()
}

fn unique_tools(sequence: &[String]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    sequence
        .iter()
        .filter_map(|tool| seen.insert(tool.clone()).then_some(tool.clone()))
        .collect()
}

fn stable_task_fingerprint(task_summary: &str) -> String {
    stable_experience_id(task_summary, &[], ExperienceOutcome::Success)
}

fn error_class_from_summary(summary: &str) -> String {
    summary
        .split('(')
        .nth(1)
        .and_then(|rest| rest.split(')').next())
        .filter(|class| !class.trim().is_empty())
        .unwrap_or("error")
        .trim()
        .to_string()
}

fn truncate_chars(input: &str, max_chars: usize) -> String {
    input.chars().take(max_chars).collect()
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::agent::hooks::{PostTurnHook, ToolCallRecord, TurnContext};
    use crate::openhuman::agent_experience::store::AgentExperienceStore;
    use crate::openhuman::agent_experience::types::ExperienceOutcome;
    use crate::openhuman::memory::Memory;
    use crate::openhuman::memory_tools::test_helpers::MockMemory;
    use std::sync::Arc;

    fn ctx_with(tool_calls: Vec<ToolCallRecord>) -> TurnContext {
        TurnContext {
            user_message: "Search the repository docs before opening the target file.".into(),
            assistant_response: "I found the docs and used the target file.".into(),
            tool_calls,
            turn_duration_ms: 1200,
            session_id: Some("session-1".into()),
            agent_id: Some("orchestrator".into()),
            entrypoint: Some("web_channel".into()),
            iteration_count: 2,
        }
    }

    fn call(name: &str, success: bool, output_summary: &str) -> ToolCallRecord {
        ToolCallRecord {
            name: name.into(),
            arguments: serde_json::json!({}),
            success,
            output_summary: output_summary.into(),
            duration_ms: 10,
        }
    }

    #[test]
    fn extract_candidates_records_successful_multi_tool_sequence() {
        let ctx = ctx_with(vec![
            call("grep", true, "grep: ok (20 chars)"),
            call("file_read", true, "file_read: ok (100 chars)"),
        ]);

        let candidates = AgentExperienceCaptureHook::extract_candidates(&ctx);

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].outcome, ExperienceOutcome::Success);
        assert_eq!(candidates[0].tool_sequence, vec!["grep", "file_read"]);
        assert_eq!(candidates[0].agent_id.as_deref(), Some("orchestrator"));
        assert_eq!(candidates[0].entrypoint.as_deref(), Some("web_channel"));
        assert!(candidates[0].lesson.contains("grep -> file_read"));
        assert!(candidates[0].tags.contains(&"multi-tool-success".into()));
    }

    #[test]
    fn extract_candidates_records_repeated_failures() {
        let ctx = ctx_with(vec![
            call("shell", false, "shell: failed (permission_denied)"),
            call("shell", false, "shell: failed (permission_denied)"),
            call("grep", true, "grep: ok (10 chars)"),
        ]);

        let candidates = AgentExperienceCaptureHook::extract_candidates(&ctx);
        let repeated_failure = candidates
            .iter()
            .find(|candidate| candidate.tags.contains(&"repeated-failure".into()))
            .expect("repeated failure candidate");

        assert_eq!(repeated_failure.outcome, ExperienceOutcome::Failure);
        assert_eq!(
            repeated_failure.error_class.as_deref(),
            Some("permission_denied")
        );
        assert!(repeated_failure.lesson.contains("shell failed 2 times"));
        assert!(repeated_failure.avoid_hint.is_some());
    }

    #[tokio::test]
    async fn on_turn_complete_persists_candidates() {
        let memory: Arc<dyn Memory> = Arc::new(MockMemory::default());
        let store = AgentExperienceStore::new(memory.clone());
        let hook = AgentExperienceCaptureHook::from_store(store.clone(), true);

        hook.on_turn_complete(&ctx_with(vec![
            call("grep", true, "grep: ok (20 chars)"),
            call("file_read", true, "file_read: ok (100 chars)"),
        ]))
        .await
        .unwrap();

        let stored = store.list().await.unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].outcome, ExperienceOutcome::Success);
        assert_eq!(stored[0].agent_id.as_deref(), Some("orchestrator"));
        assert_eq!(stored[0].entrypoint.as_deref(), Some("web_channel"));
    }
}
