//! Task execution — dispatches tasks to either the local Ollama model (text-only)
//! or the full agentic loop (tool-required).
//!
//! When agentic-v1 is used for a task that didn't have explicit write intent,
//! it runs in analysis-only mode. If it recommends a write action, execution
//! is paused and an `UnapprovedWrite` result is returned so the engine can
//! create an escalation for user approval.

use super::prompt;
use super::types::{ExecutionResult, SubconsciousTask};
use tracing::{debug, info, warn};

/// Outcome of executing a task — either completed or needs user approval.
pub enum ExecutionOutcome {
    /// Task completed (either read-only analysis or approved write).
    Completed(ExecutionResult),
    /// agentic-v1 recommends a write action on a read-only task.
    /// Contains the recommended action description for the escalation.
    UnapprovedWrite {
        recommendation: String,
        duration_ms: u64,
    },
}

/// Execute a task. Routes to local model or agentic loop based on whether
/// the task needs external tools.
pub async fn execute_task(
    task: &SubconsciousTask,
    situation_report: &str,
    identity_context: &str,
) -> Result<ExecutionOutcome, String> {
    let started = std::time::Instant::now();
    let task_has_write_intent = needs_tools(&task.title);

    let result = if task_has_write_intent {
        // Task explicitly asks for a write action — run with full permissions.
        info!(
            "[subconscious:executor] write task: {} — agentic loop, full permissions",
            task.title
        );
        execute_with_agent_full(task, situation_report, identity_context)
            .await
            .map(|output| {
                ExecutionOutcome::Completed(ExecutionResult {
                    output,
                    used_tools: true,
                    duration_ms: started.elapsed().as_millis() as u64,
                })
            })
    } else if needs_agent(&task.title) {
        // Read-only task but needs deeper reasoning — run analysis-only.
        info!(
            "[subconscious:executor] read-only task escalated: {} — agentic loop, analysis only",
            task.title
        );
        let output = execute_with_agent_analysis(task, situation_report, identity_context).await?;
        let duration_ms = started.elapsed().as_millis() as u64;

        if let Some(recommendation) = extract_recommended_action(&output) {
            // agentic-v1 wants to take a write action the user didn't ask for.
            Ok(ExecutionOutcome::UnapprovedWrite {
                recommendation,
                duration_ms,
            })
        } else {
            Ok(ExecutionOutcome::Completed(ExecutionResult {
                output,
                used_tools: false,
                duration_ms,
            }))
        }
    } else {
        // Simple text-only task — local model handles it.
        debug!(
            "[subconscious:executor] text task: {} — using local model",
            task.title
        );
        execute_with_local_model(task, situation_report, identity_context)
            .await
            .map(|output| {
                ExecutionOutcome::Completed(ExecutionResult {
                    output,
                    used_tools: false,
                    duration_ms: started.elapsed().as_millis() as u64,
                })
            })
    };

    if let Err(ref e) = result {
        warn!("[subconscious:executor] task '{}' failed: {e}", task.title);
    }

    result
}

/// Execute an approved write action — called after user approves an escalation
/// that originated from `UnapprovedWrite`.
pub async fn execute_approved_write(
    task: &SubconsciousTask,
    situation_report: &str,
    identity_context: &str,
) -> Result<ExecutionResult, String> {
    let started = std::time::Instant::now();
    let output = execute_with_agent_full(task, situation_report, identity_context).await?;
    Ok(ExecutionResult {
        output,
        used_tools: true,
        duration_ms: started.elapsed().as_millis() as u64,
    })
}

/// Execute a text-only task using the local Ollama model.
async fn execute_with_local_model(
    task: &SubconsciousTask,
    situation_report: &str,
    identity_context: &str,
) -> Result<String, String> {
    let config = crate::openhuman::config::Config::load_or_init()
        .await
        .map_err(|e| format!("config load: {e}"))?;

    let prompt_text = prompt::build_text_execution_prompt(task, situation_report, identity_context);

    let messages = vec![
        crate::openhuman::local_ai::ops::LocalAiChatMessage {
            role: "system".to_string(),
            content: prompt_text,
        },
        crate::openhuman::local_ai::ops::LocalAiChatMessage {
            role: "user".to_string(),
            content: "Execute the task now.".to_string(),
        },
    ];

    let outcome = crate::openhuman::local_ai::ops::local_ai_chat(&config, messages, None)
        .await
        .map_err(|e| format!("local model: {e}"))?;

    Ok(outcome.value)
}

/// Execute with agentic-v1 at full permissions (write-intent tasks or approved writes).
///
/// Retries up to 3 times with exponential backoff (2s, 4s, 8s) on 429 rate-limit
/// errors from the agentic-v1 cloud model.
async fn execute_with_agent_full(
    task: &SubconsciousTask,
    situation_report: &str,
    identity_context: &str,
) -> Result<String, String> {
    let mut config = crate::openhuman::config::Config::load_or_init()
        .await
        .map_err(|e| format!("config load: {e}"))?;

    let prompt_text = prompt::build_tool_execution_prompt(task, situation_report, identity_context);

    agent_chat_with_retry(&mut config, &prompt_text).await
}

/// Execute with agentic-v1 in analysis-only mode (read-only tasks).
///
/// The prompt instructs the model to analyze but not execute write actions.
async fn execute_with_agent_analysis(
    task: &SubconsciousTask,
    situation_report: &str,
    identity_context: &str,
) -> Result<String, String> {
    let mut config = crate::openhuman::config::Config::load_or_init()
        .await
        .map_err(|e| format!("config load: {e}"))?;

    let prompt_text = prompt::build_analysis_only_prompt(task, situation_report, identity_context);

    agent_chat_with_retry(&mut config, &prompt_text).await
}

/// Call agent_chat with rate-limit retry (429 only, up to 3 attempts).
async fn agent_chat_with_retry(
    config: &mut crate::openhuman::config::Config,
    prompt: &str,
) -> Result<String, String> {
    const MAX_RETRIES: u32 = 3;
    let mut attempt = 0;

    loop {
        let result =
            crate::openhuman::local_ai::ops::agent_chat(config, prompt, None, Some(0.3)).await;

        match result {
            Ok(outcome) => return Ok(outcome.value),
            Err(e) => {
                let is_rate_limit = e.contains("429") || e.to_lowercase().contains("rate limit");
                attempt += 1;

                if is_rate_limit && attempt < MAX_RETRIES {
                    let backoff_secs = 2u64 << (attempt - 1); // 2, 4, 8
                    warn!(
                        "[subconscious:executor] rate-limited (attempt {}/{}), retrying in {}s: {}",
                        attempt, MAX_RETRIES, backoff_secs, e
                    );
                    tokio::time::sleep(std::time::Duration::from_secs(backoff_secs)).await;
                    continue;
                }

                return Err(format!("agent execution: {e}"));
            }
        }
    }
}

/// Check if the analysis output contains a recommended write action.
/// Returns the recommendation text if found.
fn extract_recommended_action(output: &str) -> Option<String> {
    // Look for "RECOMMENDED ACTION:" marker in the output
    for line_idx in output.lines().enumerate().filter_map(|(i, l)| {
        if l.trim().starts_with("RECOMMENDED ACTION:") {
            Some(i)
        } else {
            None
        }
    }) {
        let recommendation: String = output
            .lines()
            .skip(line_idx)
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_string();
        if !recommendation.is_empty() {
            return Some(recommendation);
        }
    }
    None
}

/// Heuristic: does this task need the agentic loop (deeper reasoning, tools)?
///
/// Tasks escalated by the local model that involve complex analysis
/// (multi-step reasoning, cross-referencing sources) benefit from agentic-v1
/// even without write actions.
fn needs_agent(title: &str) -> bool {
    let lower = title.to_lowercase();
    let agent_keywords = [
        "compare",
        "cross-reference",
        "correlate",
        "investigate",
        "deep dive",
        "research",
        "audit",
        "trace",
        "debug",
        "diagnose",
    ];
    agent_keywords.iter().any(|kw| lower.contains(kw))
}

/// Heuristic: does this task description imply needing external tools?
///
/// Tasks with action verbs (send, create, post, delete, move, publish, schedule)
/// need the agentic loop. Tasks with passive verbs (summarize, check, monitor,
/// review, analyze, extract, classify) can be handled by local model.
pub fn needs_tools(title: &str) -> bool {
    let lower = title.to_lowercase();
    let tool_keywords = [
        "send",
        "post",
        "create",
        "delete",
        "remove",
        "move",
        "publish",
        "schedule",
        "forward",
        "reply",
        "draft and send",
        "upload",
        "download",
        "notify on",
        "alert on",
        "message",
        "write to",
        "update on",
        "sync to",
    ];
    tool_keywords.iter().any(|kw| lower.contains(kw))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn needs_tools_detects_action_verbs() {
        assert!(needs_tools("Send email digest to Telegram"));
        assert!(needs_tools("Post weekly standup to Slack"));
        assert!(needs_tools("Create a summary in Notion"));
        assert!(needs_tools("Delete old calendar events"));
        assert!(needs_tools("Forward urgent emails to team"));
        assert!(needs_tools("Schedule a meeting for tomorrow"));
    }

    #[test]
    fn needs_tools_rejects_passive_verbs() {
        assert!(!needs_tools("Summarize unread emails"));
        assert!(!needs_tools("Check skills runtime health"));
        assert!(!needs_tools("Monitor Ollama status"));
        assert!(!needs_tools("Review upcoming deadlines"));
        assert!(!needs_tools("Analyze email patterns"));
        assert!(!needs_tools("Extract key points from Notion pages"));
        assert!(!needs_tools("Classify email priority"));
    }

    #[test]
    fn needs_tools_case_insensitive() {
        assert!(needs_tools("SEND a message to Slack"));
        assert!(needs_tools("Send A Message To Slack"));
    }

    #[test]
    fn needs_agent_detects_complex_tasks() {
        assert!(needs_agent("Compare Q1 and Q2 revenue data"));
        assert!(needs_agent("Investigate why notifications stopped"));
        assert!(needs_agent("Audit all active skill connections"));
        assert!(!needs_agent("Check emails"));
        assert!(!needs_agent("Summarize today's events"));
    }

    #[test]
    fn extract_recommended_action_finds_marker() {
        let output = "Analysis complete. Found 3 urgent emails.\n\nRECOMMENDED ACTION: Forward the 3 urgent emails to #team-alerts on Slack.";
        let action = extract_recommended_action(output);
        assert!(action.is_some());
        assert!(action.unwrap().contains("Forward"));
    }

    #[test]
    fn extract_recommended_action_returns_none_when_absent() {
        let output = "All skills are healthy. No issues found.";
        assert!(extract_recommended_action(output).is_none());
    }
}
