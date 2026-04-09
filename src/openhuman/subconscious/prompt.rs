//! Prompt builders for the subconscious evaluation and execution phases.
//!
//! Injects OpenClaw identity context (SOUL.md, USER.md) so the local model
//! reasons as the agent, not a generic evaluator.

use super::types::SubconsciousTask;
use std::path::Path;

const IDENTITY_EXCERPT_CHARS: usize = 2000;

// ── Evaluation prompt ────────────────────────────────────────────────────────

/// Build the per-tick evaluation prompt. The local model evaluates each due
/// task against the situation report and returns a per-task decision.
pub fn build_evaluation_prompt(
    tasks: &[SubconsciousTask],
    situation_report: &str,
    identity_context: &str,
) -> String {
    let task_list = tasks
        .iter()
        .map(|t| format!("- [{}] {}", t.id, t.title))
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        r#"{identity_context}

# Subconscious Loop — Task Evaluation

You are the background awareness layer. You run periodically to evaluate
user-defined tasks against the current workspace state.

## Due tasks

{task_list}

## Current state

{situation_report}

## Your job

For each task, check if the current state has anything relevant. Decide:
- **noop**: Nothing actionable for this task right now.
- **act**: The task should be executed now (state has relevant data).
- **escalate**: The task needs user approval before acting (ambiguous, risky, or irreversible).

## Output format (strict JSON, no other text)

{{
  "evaluations": [
    {{"task_id": "<id>", "decision": "noop|act|escalate", "reason": "one sentence"}}
  ]
}}
"#
    )
}

// ── Execution prompts ────────────────────────────────────────────────────────

/// Build the prompt for executing a text-only task via local Ollama model.
/// Used for tasks that don't need tools (summarize, extract, classify, etc.)
pub fn build_text_execution_prompt(
    task: &SubconsciousTask,
    situation_report: &str,
    identity_context: &str,
) -> String {
    format!(
        r#"{identity_context}

# Task Execution

Execute the following task based on the current state. Respond with the result only.

## Task
{task_title}

## Current state
{situation_report}

Do the task now. Return only the result — no explanations or meta-commentary."#,
        task_title = task.title
    )
}

/// Build the prompt for executing a tool-required task via the full agentic loop.
/// Used for tasks that need side effects (send message, create doc, etc.)
pub fn build_tool_execution_prompt(
    task: &SubconsciousTask,
    situation_report: &str,
    identity_context: &str,
) -> String {
    format!(
        r#"{identity_context}

# Background Task Execution

You are executing a user-defined background task. Use your available tools to complete it.

## Task
{task_title}

## Current state
{situation_report}

Execute this task using the appropriate tools. Complete the task fully — don't just describe what to do."#,
        task_title = task.title
    )
}

/// Build a read-only analysis prompt for agentic-v1. Used when a read-only task
/// is escalated — the agent should analyze and recommend but NOT execute writes.
pub fn build_analysis_only_prompt(
    task: &SubconsciousTask,
    situation_report: &str,
    identity_context: &str,
) -> String {
    format!(
        r#"{identity_context}

# Background Task — Analysis Only

You are analyzing a background task. You may use read-only tools to gather information,
but you MUST NOT execute any write actions (send, post, create, delete, forward, reply, update, publish).

If you determine that a write action is needed, describe exactly what you would do in your response
but do not execute it. Start your recommendation with "RECOMMENDED ACTION:" on its own line.

## Task
{task_title}

## Current state
{situation_report}

Analyze the situation and report your findings. If action is needed, describe it clearly but do NOT execute."#,
        task_title = task.title
    )
}

// ── Identity loading ─────────────────────────────────────────────────────────

/// Load identity context from SOUL.md and USER.md in the prompts directory.
/// Returns a formatted string to prepend to prompts.
pub fn load_identity_context(workspace_dir: &Path) -> String {
    let prompts_dir = resolve_prompts_dir(workspace_dir);
    let mut ctx = String::new();

    if let Some(ref dir) = prompts_dir {
        if let Some(soul) = load_file_excerpt(dir, "SOUL.md") {
            ctx.push_str(&soul);
            ctx.push_str("\n\n");
        }
        if let Some(user) = load_file_excerpt(dir, "USER.md") {
            ctx.push_str("## User Context\n\n");
            ctx.push_str(&user);
            ctx.push_str("\n\n");
        }
    }

    if ctx.is_empty() {
        "You are OpenHuman, an AI assistant for productivity and collaboration.".to_string()
    } else {
        ctx
    }
}

fn resolve_prompts_dir(workspace_dir: &Path) -> Option<std::path::PathBuf> {
    // Check workspace AI dir
    let workspace_ai = workspace_dir.join("ai");
    if workspace_ai.is_dir() {
        return Some(workspace_ai);
    }

    // Try CARGO_MANIFEST_DIR (dev builds)
    if let Some(dir) = option_env!("CARGO_MANIFEST_DIR").map(std::path::PathBuf::from) {
        let candidate = dir
            .join("src")
            .join("openhuman")
            .join("agent")
            .join("prompts");
        if candidate.is_dir() {
            return Some(candidate);
        }
    }

    // Walk up from cwd
    if let Ok(cwd) = std::env::current_dir() {
        return crate::openhuman::dev_paths::repo_ai_prompts_dir(&cwd);
    }

    None
}

fn load_file_excerpt(dir: &Path, filename: &str) -> Option<String> {
    let content = std::fs::read_to_string(dir.join(filename)).ok()?;
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.chars().count() > IDENTITY_EXCERPT_CHARS {
        let truncated: String = trimmed.chars().take(IDENTITY_EXCERPT_CHARS).collect();
        Some(format!("{truncated}\n[... truncated]"))
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::subconscious::types::{TaskRecurrence, TaskSource};

    fn test_task(id: &str, title: &str) -> SubconsciousTask {
        SubconsciousTask {
            id: id.to_string(),
            title: title.to_string(),
            source: TaskSource::User,
            recurrence: TaskRecurrence::Once,
            enabled: true,
            last_run_at: None,
            next_run_at: None,
            completed: false,
            created_at: 0.0,
        }
    }

    #[test]
    fn evaluation_prompt_includes_tasks_and_report() {
        let tasks = vec![
            test_task("t1", "Check email"),
            test_task("t2", "Review calendar"),
        ];
        let prompt = build_evaluation_prompt(&tasks, "## State\nSome data.", "Identity here");
        assert!(prompt.contains("[t1] Check email"));
        assert!(prompt.contains("[t2] Review calendar"));
        assert!(prompt.contains("Some data."));
        assert!(prompt.contains("Identity here"));
    }

    #[test]
    fn evaluation_prompt_includes_decision_schema() {
        let tasks = vec![test_task("t1", "Task")];
        let prompt = build_evaluation_prompt(&tasks, "", "");
        assert!(prompt.contains("noop"));
        assert!(prompt.contains("act"));
        assert!(prompt.contains("escalate"));
        assert!(prompt.contains("evaluations"));
        assert!(prompt.contains("task_id"));
    }

    #[test]
    fn text_execution_prompt_includes_task_title() {
        let task = test_task("t1", "Summarize urgent emails");
        let prompt = build_text_execution_prompt(&task, "3 new emails", "Identity");
        assert!(prompt.contains("Summarize urgent emails"));
        assert!(prompt.contains("3 new emails"));
    }

    #[test]
    fn tool_execution_prompt_includes_tool_instructions() {
        let task = test_task("t1", "Send digest to Telegram");
        let prompt = build_tool_execution_prompt(&task, "Email data here", "Identity");
        assert!(prompt.contains("Send digest to Telegram"));
        assert!(prompt.contains("tools"));
    }

    #[test]
    fn identity_context_loads_or_falls_back() {
        let ctx = load_identity_context(std::path::Path::new("/nonexistent"));
        assert!(ctx.contains("OpenHuman"));
    }
}
