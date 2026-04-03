//! System prompt for the subconscious local-model evaluation.

/// Build the task-driven system prompt for the subconscious tick.
///
/// The local model evaluates each HEARTBEAT.md task against the current
/// state and decides which tasks have actionable items right now.
pub fn build_subconscious_prompt(tasks: &[String], situation_report: &str) -> String {
    let task_list = tasks
        .iter()
        .enumerate()
        .map(|(i, t)| format!("{}. {}", i + 1, t))
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        r#"# Subconscious Loop — Task Evaluation

You are the background awareness layer of OpenHuman. You run periodically
to check a list of user-defined tasks against the current workspace state.

## Your tasks to check

{task_list}

## Current state

{situation_report}

## Your job

For each task above, check if the current state contains anything relevant.
If a task has something actionable, include it in the actions list.
If no tasks have anything actionable, return noop.

## Output format (strict JSON, no other text)

{{
  "decision": "noop" | "act" | "escalate",
  "reason": "one sentence summary of what you found",
  "actions": [
    {{
      "type": "notify" | "store_memory" | "escalate_to_agent",
      "description": "what was found and what to do about it",
      "priority": "low" | "medium" | "high",
      "task": "which task this relates to"
    }}
  ]
}}

## Decision rules

- **noop**: None of the tasks have anything actionable in the current state.
- **act**: One or more tasks have findings that can be summarized as a notification or stored as a memory note.
- **escalate**: A task finding requires complex reasoning or multi-step action that the full agent should handle (e.g. drafting a response, reprioritizing work, multi-tool operations).

## Examples

Task: "Check email for urgent items"
State shows: new email about deadline moved to tomorrow
→ act, notify with high priority

Task: "Monitor skills runtime health"
State shows: no skill data available
→ noop for this task

Task: "Check for deadline changes"
State shows: project tracker updated, 3 deadlines shifted, ownership changed
→ escalate (too complex for simple notification)
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_includes_tasks_and_report() {
        let tasks = vec!["Check email".to_string(), "Review calendar".to_string()];
        let prompt = build_subconscious_prompt(&tasks, "## Test\n\nSome data.");
        assert!(prompt.contains("1. Check email"));
        assert!(prompt.contains("2. Review calendar"));
        assert!(prompt.contains("## Test"));
        assert!(prompt.contains("Some data."));
    }

    #[test]
    fn prompt_includes_json_schema() {
        let prompt = build_subconscious_prompt(&["Task".into()], "");
        assert!(prompt.contains("noop"));
        assert!(prompt.contains("escalate"));
        assert!(prompt.contains("escalate_to_agent"));
    }

    #[test]
    fn prompt_includes_task_field_in_actions() {
        let prompt = build_subconscious_prompt(&["Task".into()], "");
        assert!(prompt.contains("\"task\""));
    }
}
