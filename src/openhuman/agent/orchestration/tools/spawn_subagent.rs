//! Tool: `spawn_subagent` — delegate a sub-task to a specialised sub-agent.
//!
//! The orchestrator (or any parent agent that has this tool registered)
//! calls `spawn_subagent` to hand off a focused sub-task. The runner
//! looks up the requested [`AgentDefinition`] in the global registry,
//! filters the parent's tool registry per the definition, builds a
//! narrow system prompt, and runs an inner tool-call loop using the
//! parent's provider. The sub-agent's intra-loop history is collapsed
//! into a single text result that the parent receives as a normal
//! `tool_result`.
//!
//! Sub-agents always run in "typed" mode: a narrow archetype-specific
//! prompt with a filtered tool list, on a cheaper model where applicable.
//!
use crate::openhuman::agent::harness::definition::AgentDefinitionRegistry;
use crate::openhuman::agent::harness::fork_context::current_parent;
use crate::openhuman::agent::harness::subagent_runner::{
    run_subagent, SubagentRunOptions, SubagentRunOutcome, SubagentRunStatus,
};
use crate::openhuman::agent::progress::AgentProgress;
use crate::openhuman::memory::conversations::{
    self as conversations, ConversationMessage, CreateConversationThread,
};
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolCallOptions, ToolResult};
use async_trait::async_trait;
use serde_json::json;
use std::path::PathBuf;
use tinytools::ToolRunContext;

/// Spawns a sub-agent of the requested type to handle a delegated task.
///
/// Registered into the parent agent's tool list by
/// [`crate::openhuman::tools::all_tools_with_runtime`]. The orchestrator
/// archetype's tool whitelist already includes `spawn_subagent`, so
/// orchestrated runs see it; non-orchestrator parents see it too unless
/// explicitly removed.
pub struct SpawnSubagentTool;

impl Default for SpawnSubagentTool {
    fn default() -> Self {
        Self::new()
    }
}

impl SpawnSubagentTool {
    pub fn new() -> Self {
        Self
    }

    fn classify_subagent_failure(message: &str) -> String {
        let lower = message.to_lowercase();
        let upstream_unhealthy = lower.contains("no healthy upstream")
            || lower.contains("upstream_unhealthy")
            || lower.contains("upstream unavailable")
            || lower.contains("service unavailable")
            || lower.contains("provider call failed: all providers/models failed");

        if upstream_unhealthy {
            return format!(
                "spawn_subagent failed: upstream inference unavailable \
                 (LLM provider outage/capacity). This is NOT a Composio/integration auth issue. \
                 Avoid immediate repeated retries; ask user to retry shortly.\nDetails: {message}"
            );
        }

        format!("spawn_subagent failed: {message}")
    }
}

include!("spawn_subagent_tool_impl.rs");

/// Trim a raw prompt down to a thread-list-friendly title.
///
/// Mirrors the visible-character cap the UI threads list uses so titles
/// stay readable when the orchestrator hands in a multi-paragraph prompt.
const WORKER_THREAD_TITLE_MAX_CHARS: usize = 80;

fn build_worker_thread_title(prompt: &str) -> String {
    let collapsed: String = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return "Worker task".to_string();
    }
    let mut iter = collapsed.chars();
    let truncated: String = iter.by_ref().take(WORKER_THREAD_TITLE_MAX_CHARS).collect();
    if iter.next().is_some() {
        format!("{truncated}…")
    } else {
        truncated
    }
}

fn persist_worker_thread(
    workspace_dir: &std::path::Path,
    agent_id: &str,
    prompt: &str,
    outcome: &SubagentRunOutcome,
) -> Result<String, String> {
    let thread_id = format!("worker-{}", uuid::Uuid::new_v4());
    let title = build_worker_thread_title(prompt);
    let now = chrono::Utc::now().to_rfc3339();

    conversations::ensure_thread(
        workspace_dir.to_path_buf(),
        CreateConversationThread {
            id: thread_id.clone(),
            title,
            created_at: now.clone(),
            parent_thread_id: None,
            labels: Some(vec!["tasks".to_string()]),
            personality_id: None,
        },
    )
    .map_err(|err| format!("ensure_thread: {err}"))?;

    conversations::append_message(
        workspace_dir.to_path_buf(),
        &thread_id,
        ConversationMessage {
            id: format!("user:{}", outcome.task_id),
            content: prompt.to_string(),
            message_type: "text".to_string(),
            extra_metadata: json!({
                "scope": "worker_thread",
                "agent_id": agent_id,
                "task_id": outcome.task_id,
            }),
            sender: "user".to_string(),
            created_at: now.clone(),
        },
    )
    .map_err(|err| format!("append user message: {err}"))?;

    conversations::append_message(
        workspace_dir.to_path_buf(),
        &thread_id,
        ConversationMessage {
            id: format!("agent:{}", outcome.task_id),
            content: outcome.output.clone(),
            message_type: "text".to_string(),
            extra_metadata: json!({
                "scope": "worker_thread",
                "agent_id": outcome.agent_id,
                "task_id": outcome.task_id,
                "elapsed_ms": outcome.elapsed.as_millis() as u64,
                "iterations": outcome.iterations,
            }),
            sender: "agent".to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
        },
    )
    .map_err(|err| format!("append agent message: {err}"))?;

    Ok(thread_id)
}

/// Build a parent-thread tool_result that refers the user to the worker
/// thread instead of dumping the sub-agent's full transcript inline.
///
/// The `[worker_thread_ref] … [/worker_thread_ref]` envelope carries
/// machine-readable metadata the UI parses to render a clickable card; the
/// surrounding prose stays informative for the LLM that reads the result.
fn render_worker_thread_result(
    thread_id: &str,
    agent_id: &str,
    outcome: &SubagentRunOutcome,
) -> String {
    let payload = json!({
        "thread_id": thread_id,
        "label": "worker",
        "agent_id": agent_id,
        "task_id": outcome.task_id,
        "elapsed_ms": outcome.elapsed.as_millis() as u64,
        "iterations": outcome.iterations,
    });
    format!(
        "Spawned worker thread `{thread_id}` for the delegated task. The \
         user can open it from the thread list (label: `worker`) to see \
         the sub-agent's full transcript. Continue from a brief summary \
         in this thread instead of relaying the entire run.\n\n\
         [worker_thread_ref]\n{payload}\n[/worker_thread_ref]",
        thread_id = thread_id,
        payload = serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string()),
    )
}

/// Build the user-facing explanation for an allowlisted-but-not-active
/// integration during an `integrations_agent` spawn (#2365).
///
/// The single message that previously covered every cause ("available
/// but the user has not authorized it yet") looked confused to users
/// who had Gmail showing in Settings (because Settings reflects the
/// FE's optimistic post-OAuth view, while the spawn gate reads the
/// backend's authoritative status). We now pivot on the upstream
/// connection status:
///
/// - `INITIATED` / `INITIALIZING` / `PENDING` — OAuth in progress;
///   ask the user to finish the flow in their browser.
/// - `EXPIRED` — token rolled over; reconnect.
/// - `FAILED` / `ERROR` — handshake didn't land; reconnect.
/// - any other non-active status — quote the upstream verbatim.
/// - `None` — no connection row at all (truly disconnected).
///
/// Returns text the model reads literally; the orchestrator paraphrases
/// it into a user-facing reply. Keep the *intent* stable across
/// rewordings — the "Connections → {toolkit}" path is
/// load-bearing for the UI navigation tests.
pub(crate) fn describe_unconnected_state(toolkit: &str, status: Option<&str>) -> String {
    // Keep the original (trimmed) status separately so the
    // unknown-status branch can quote it verbatim — CodeRabbit
    // review on #2373: matching on the uppercased value AND
    // formatting with that uppercased value broke the
    // "quote upstream status verbatim" contract for mixed/lowercase
    // wire shapes.
    let trimmed = status.map(str::trim).filter(|s| !s.is_empty());
    let upper = trimmed.map(|s| s.to_ascii_uppercase());
    match upper.as_deref() {
        Some("INITIATED") | Some("INITIALIZING") | Some("PENDING") => format!(
            "Integration '{toolkit}' has an OAuth flow in progress but it hasn't reached \
             ACTIVE yet. Do NOT retry this spawn. Tell the user the authorization is \
             pending and ask them to finish the browser OAuth flow (Connections → \
             '{toolkit}') before retrying. If they already closed the \
             browser tab, they can restart the connection from the same Connections page."
        ),
        Some("EXPIRED") => format!(
            "Integration '{toolkit}' is connected but the OAuth token has expired. \
             Do NOT retry this spawn. Tell the user the connection expired and ask \
             them to reconnect '{toolkit}' at Connections → '{toolkit}' \
             before retrying the original request."
        ),
        Some("FAILED") | Some("ERROR") => {
            // Quote the actual upstream label (FAILED / ERROR) instead of
            // hard-coding "FAILED" — triage cross-references backend logs
            // and a misquoted `ERROR` row showing up as "FAILED" wastes
            // their time. graycyrus review on #2373.
            let raw = trimmed.unwrap_or("");
            format!(
                "Integration '{toolkit}' has a previous OAuth attempt in a `{raw}` state. \
                 Do NOT retry this spawn. Tell the user the connection failed and ask them \
                 to reconnect '{toolkit}' at Connections → '{toolkit}' before \
                 retrying the original request."
            )
        }
        Some(_) => {
            // Quote the *original* upstream status, not its uppercased
            // form — preserves "DeauthRequired" / "needs_relink"-style
            // mixed-case wire values for triage.
            let raw = trimmed.unwrap_or("");
            format!(
                "Integration '{toolkit}' has a connection row but its status is `{raw}`, \
                 which is not yet usable. Do NOT retry this spawn. Tell the user the \
                 connection is in an unusable state and ask them to reconnect '{toolkit}' \
                 at Connections → '{toolkit}'."
            )
        }
        _ => format!(
            "Integration '{toolkit}' is available but the user has not authorized it \
             yet. Do NOT retry this spawn. Tell the user the integration is available \
             and ask them to authorize '{toolkit}' in Connections → \
             '{toolkit}' before retrying the original request."
        ),
    }
}

#[cfg(test)]
#[path = "spawn_subagent_tests.rs"]
mod tests;
