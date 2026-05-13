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

#[cfg(test)]
mod test_mocks {
    use std::sync::atomic::{AtomicU8, Ordering};

    const MODE_REAL: u8 = 0;
    const MODE_LOCAL_FAIL: u8 = 1;
    const MODE_AGENT_FAIL: u8 = 2;

    static MODE: AtomicU8 = AtomicU8::new(MODE_REAL);

    pub fn mock_local() {
        MODE.store(MODE_LOCAL_FAIL, Ordering::Release);
    }
    pub fn mock_agent() {
        MODE.store(MODE_AGENT_FAIL, Ordering::Release);
    }
    pub fn reset() {
        MODE.store(MODE_REAL, Ordering::Release);
    }
    pub fn is_local_mocked() -> bool {
        MODE.load(Ordering::Acquire) == MODE_LOCAL_FAIL
    }
    pub fn is_agent_mocked() -> bool {
        MODE.load(Ordering::Acquire) == MODE_AGENT_FAIL
    }
}

/// Outcome of executing a task — either completed or needs user approval.
#[derive(Debug)]
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
    let mut config = crate::openhuman::config::Config::load_or_init()
        .await
        .map_err(|e| format!("config load: {e}"))?;

    let result = if task_has_write_intent {
        // Task explicitly asks for a write action — run with full permissions.
        info!(
            "[subconscious:executor] write task: id={} — agentic loop, full permissions",
            task.id
        );
        execute_with_agent_full(&mut config, task, situation_report, identity_context)
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
            "[subconscious:executor] read-only task escalated: id={} — agentic loop, analysis only",
            task.id
        );
        let output =
            execute_with_agent_analysis(&mut config, task, situation_report, identity_context)
                .await?;
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
        // Simple text-only task. Use local model if configured for subconscious
        // tasks, otherwise fall back to the cloud agentic analysis path.
        if config.local_ai.use_local_for_subconscious() {
            debug!(
                "[subconscious:executor] text task: id={} — using local model",
                task.id
            );
            execute_with_local_model(&config, task, situation_report, identity_context)
                .await
                .map(|output| {
                    ExecutionOutcome::Completed(ExecutionResult {
                        output,
                        used_tools: false,
                        duration_ms: started.elapsed().as_millis() as u64,
                    })
                })
        } else {
            info!(
                "[subconscious:executor] text task: id={} — local AI disabled, using cloud fallback",
                task.id
            );
            let output =
                execute_with_agent_analysis(&mut config, task, situation_report, identity_context)
                    .await
                    .map_err(|e| format!("cloud fallback agent execution: {e}"))?;
            let duration_ms = started.elapsed().as_millis() as u64;
            debug!(
                "[subconscious:executor] text task cloud fallback complete: id={} — duration_ms={}",
                task.id, duration_ms
            );

            // Suppress UnapprovedWrite: passive tasks that didn't trigger
            // needs_agent should never escalate even if the cloud model's
            // output contains RECOMMENDED ACTION. The write-intent gate is
            // needs_tools for active tasks and needs_agent for read-only
            // escalations; the cloud fallback is a passthrough for simple
            // text tasks and must not silently change the contract.
            Ok(ExecutionOutcome::Completed(ExecutionResult {
                output,
                used_tools: false,
                duration_ms,
            }))
        }
    };

    if let Err(ref e) = result {
        warn!("[subconscious:executor] task id={} failed: {e}", task.id);
    }

    result
}

/// Execute an approved write action — called after user approves an escalation
/// that originated from `UnapprovedWrite`.
///
/// Independent `Config::load_or_init()`: the task was originally routed under
/// config_A in `execute_task`; now executes under config_B after user approval.
/// If `use_local_for_subconscious()` toggled between the two calls, the approval
/// was made under different assumptions. Risk is negligible in practice (config
/// changes require a restart to take effect on most fields), but callers should
/// be aware of this TOCTOU window.
pub async fn execute_approved_write(
    task: &SubconsciousTask,
    situation_report: &str,
    identity_context: &str,
) -> Result<ExecutionResult, String> {
    let started = std::time::Instant::now();
    let mut config = crate::openhuman::config::Config::load_or_init()
        .await
        .map_err(|e| format!("config load: {e}"))?;
    let output =
        execute_with_agent_full(&mut config, task, situation_report, identity_context).await?;
    Ok(ExecutionResult {
        output,
        used_tools: true,
        duration_ms: started.elapsed().as_millis() as u64,
    })
}

/// Execute a text-only task using the local Ollama model.
///
/// The caller MUST have already checked `config.local_ai.use_local_for_subconscious()`
/// before calling this function.
async fn execute_with_local_model(
    config: &crate::openhuman::config::Config,
    task: &SubconsciousTask,
    situation_report: &str,
    identity_context: &str,
) -> Result<String, String> {
    #[cfg(test)]
    if test_mocks::is_local_mocked() {
        return Err("local model: mocked failure (test)".into());
    }
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
    config: &mut crate::openhuman::config::Config,
    task: &SubconsciousTask,
    situation_report: &str,
    identity_context: &str,
) -> Result<String, String> {
    let prompt_text = prompt::build_tool_execution_prompt(task, situation_report, identity_context);

    agent_chat_with_retry(config, &prompt_text).await
}

/// Execute with agentic-v1 in analysis-only mode (read-only tasks).
///
/// The prompt instructs the model to analyze but not execute write actions.
async fn execute_with_agent_analysis(
    config: &mut crate::openhuman::config::Config,
    task: &SubconsciousTask,
    situation_report: &str,
    identity_context: &str,
) -> Result<String, String> {
    #[cfg(test)]
    if test_mocks::is_agent_mocked() {
        return Err("cloud fallback: mocked failure (test)".into());
    }
    let prompt_text = prompt::build_analysis_only_prompt(task, situation_report, identity_context);

    agent_chat_with_retry(config, &prompt_text).await
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
    use std::path::Path;
    use tempfile::tempdir;

    /// Guard that sets an env var for the duration of the test and restores it on drop.
    struct EnvVarGuard {
        key: String,
        old: Option<String>,
    }

    impl EnvVarGuard {
        fn set_to_path(key: &str, value: &Path) -> Self {
            let old = std::env::var(key).ok();
            std::env::set_var(key, value.to_str().expect("path is valid utf-8"));
            Self {
                key: key.to_string(),
                old,
            }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            match &self.old {
                Some(value) => std::env::set_var(&self.key, value),
                None => std::env::remove_var(&self.key),
            }
        }
    }

    fn write_subconscious_test_config(workspace_root: &Path, local_ai_enabled: bool) {
        let cfg = format!(
            r#"default_temperature = 0.7

[local_ai]
runtime_enabled = {local_ai_enabled}
provider = "ollama"

[local_ai.usage]
subconscious = {local_ai_enabled}

[memory]
backend = "sqlite"
auto_save = true
embedding_provider = "none"
embedding_model = "none"
embedding_dimensions = 0

[secrets]
encrypt = false
"#
        );
        std::fs::create_dir_all(workspace_root).expect("mkdir test workspace root");
        let config_path = workspace_root.join("config.toml");
        std::fs::write(&config_path, &cfg).expect("write test config");
        let _: crate::openhuman::config::Config =
            toml::from_str(&cfg).expect("test config should deserialize");
    }

    fn make_text_task(title: &str) -> SubconsciousTask {
        SubconsciousTask {
            id: "test-id".into(),
            title: title.into(),
            source: super::super::types::TaskSource::User,
            recurrence: super::super::types::TaskRecurrence::Once,
            enabled: true,
            last_run_at: None,
            next_run_at: None,
            completed: false,
            created_at: 1700000000.0,
        }
    }

    #[tokio::test]
    async fn execute_task_routes_to_cloud_when_local_disabled() {
        let _env_lock = crate::openhuman::config::TEST_ENV_LOCK
            .lock()
            .expect("test env lock");
        test_mocks::reset();
        let tmp = tempdir().expect("tempdir");
        let _workspace = EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", tmp.path());
        write_subconscious_test_config(tmp.path(), false);

        test_mocks::mock_agent();
        let task = make_text_task("Summarize unread emails");
        let result = execute_task(&task, "", "").await;

        assert!(result.is_err(), "expected error (cloud path)");
        let err = result.unwrap_err();
        assert!(
            err.contains("cloud fallback"),
            "expected cloud fallback error, got: {err}"
        );
    }

    #[tokio::test]
    async fn execute_task_routes_to_local_when_local_enabled() {
        let _env_lock = crate::openhuman::config::TEST_ENV_LOCK
            .lock()
            .expect("test env lock");
        test_mocks::reset();
        let tmp = tempdir().expect("tempdir");
        let _workspace = EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", tmp.path());
        write_subconscious_test_config(tmp.path(), true);

        test_mocks::mock_local();
        let task = make_text_task("Summarize unread emails");
        let result = execute_task(&task, "", "").await;

        assert!(result.is_err(), "expected error (local path)");
        let err = result.unwrap_err();
        assert!(
            err.contains("local model"),
            "expected local model error, got: {err}"
        );
    }

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
