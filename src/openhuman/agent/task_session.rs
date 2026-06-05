//! Materialises an autonomous task-board run as a top-level "task session"
//! conversation thread, so background agent work shows up in the
//! Conversations → Tasks tab exactly like a manually-launched todo.
//!
//! A manually-run todo (`handleWorkPersonal` in the app) creates a top-level
//! thread labelled `tasks` and streams the agent turn into it. Autonomous card
//! runs (`task_dispatcher`) previously ran headless with no thread, so they
//! never appeared in chat. This module gives them the same surface:
//!
//!   1. [`create_session_thread`] — a **top-level** (`parent_thread_id: None`)
//!      thread stamped `labels: ["tasks"]`, seeded with the task prompt as the
//!      opening `user` message. The frontend `isTaskThread()` predicate then
//!      lists it in the Tasks tab next to manual runs (no UI change needed).
//!   2. [`append_final`] — appends the run's final response (or failure reason)
//!      as the closing `assistant` message after the turn completes.
//!
//! The *live* streaming (text/tool deltas + tool timeline) is not done here:
//! [`task_dispatcher::run_autonomous`](super::task_dispatcher) wires the agent's
//! `on_progress` into the web-channel `spawn_progress_bridge` with the broadcast
//! client id `"system"`, so any client viewing the thread sees the run stream in
//! real time — the same mechanism cron/welcome agents use.
//!
//! All writes are best-effort: a thread-store failure is logged and the
//! autonomous run still proceeds headless, exactly as it did before this
//! feature. The session is a surface over the run, never a gate on it.

use std::path::PathBuf;

use serde_json::json;

use crate::openhuman::agent::task_board::TaskBoardCard;
use crate::openhuman::memory_conversations::{
    self as conversations, ConversationMessage, CreateConversationThread,
};

/// Label that lands a thread in the Conversations → Tasks tab. Mirrors the
/// frontend `TASKS_TAB_VALUE`/`LEGACY_TASK_LABELS` predicate and the label
/// `worker_thread::create_worker_thread` stamps on delegation sub-threads.
const TASKS_LABEL: &str = "tasks";

/// Max chars of a session-thread title taken from the card title.
const TITLE_MAX_CHARS: usize = 80;

/// Create a top-level task-session thread for an autonomous card run and seed
/// it with the task prompt as the opening `user` message.
///
/// Returns the new thread id, or `None` if the store rejected the create
/// (best-effort: the run still proceeds headless). The `run_id`/`card_id` are
/// stamped into the seed message metadata so the session can be correlated back
/// to its board card and liveness run.
pub(crate) fn create_session_thread(
    workspace_dir: PathBuf,
    card: &TaskBoardCard,
    run_id: &str,
    prompt: &str,
) -> Option<String> {
    let thread_id = format!("task-{}", uuid::Uuid::new_v4());
    let now = chrono::Utc::now().to_rfc3339();

    if let Err(err) = conversations::ensure_thread(
        workspace_dir.clone(),
        CreateConversationThread {
            id: thread_id.clone(),
            title: session_title(card),
            created_at: now.clone(),
            parent_thread_id: None,
            labels: Some(vec![TASKS_LABEL.to_string()]),
            personality_id: None,
        },
    ) {
        tracing::warn!(
            card_id = %card.id,
            run_id = %run_id,
            error = %err,
            "[task_session] failed to create session thread (run proceeds headless)"
        );
        return None;
    }

    if let Err(err) = conversations::append_message(
        workspace_dir,
        &thread_id,
        ConversationMessage {
            id: format!("user:{}", uuid::Uuid::new_v4()),
            content: prompt.to_string(),
            message_type: "text".to_string(),
            extra_metadata: json!({
                "scope": "autonomous_task",
                "card_id": card.id,
                "run_id": run_id,
            }),
            sender: "user".to_string(),
            created_at: now,
        },
    ) {
        tracing::warn!(
            thread_id = %thread_id,
            run_id = %run_id,
            error = %err,
            "[task_session] failed to seed task prompt (continuing)"
        );
    }

    tracing::info!(
        card_id = %card.id,
        run_id = %run_id,
        thread_id = %thread_id,
        "[task_session] created top-level task session thread for autonomous run"
    );
    Some(thread_id)
}

/// Human-readable title for the session thread — the card title, trimmed and
/// clipped, with a generic fallback for an unnamed card.
fn session_title(card: &TaskBoardCard) -> String {
    let trimmed = card.title.trim();
    if trimmed.is_empty() {
        return "Autonomous task".to_string();
    }
    trimmed.chars().take(TITLE_MAX_CHARS).collect()
}

/// Append the run's final response (or failure reason) to the session thread as
/// the closing `assistant` message, so a reopened session shows the outcome.
/// No-op on an empty response. Best-effort: a store failure is logged only.
pub(crate) fn append_final(
    workspace_dir: PathBuf,
    thread_id: &str,
    outcome: &Result<String, String>,
) {
    let (content, success) = match outcome {
        Ok(text) => (text.trim().to_string(), true),
        Err(err) => (format!("Run failed: {err}"), false),
    };
    if content.is_empty() {
        return;
    }
    if let Err(err) = conversations::append_message(
        workspace_dir,
        thread_id,
        ConversationMessage {
            id: format!("assistant:{}", uuid::Uuid::new_v4()),
            content,
            message_type: "text".to_string(),
            extra_metadata: json!({ "scope": "autonomous_task_result", "success": success }),
            sender: "assistant".to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
        },
    ) {
        tracing::debug!(
            thread_id = %thread_id,
            error = %err,
            "[task_session] failed to append final response"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::agent::task_board::{TaskBoardCard, TaskCardStatus};

    fn card(title: &str) -> TaskBoardCard {
        TaskBoardCard {
            id: "card-1".to_string(),
            title: title.to_string(),
            status: TaskCardStatus::InProgress,
            objective: None,
            plan: Vec::new(),
            assigned_agent: None,
            allowed_tools: Vec::new(),
            approval_mode: None,
            acceptance_criteria: Vec::new(),
            evidence: Vec::new(),
            notes: None,
            blocker: None,
            session_thread_id: None,
            source_metadata: None,
            order: 0,
            updated_at: String::new(),
        }
    }

    fn temp_ws() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("task-session-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn creates_top_level_tasks_thread_and_seeds_prompt() {
        let ws = temp_ws();
        let id = create_session_thread(
            ws.clone(),
            &card("Design the onboarding"),
            "run-1",
            "Do the thing",
        )
        .expect("thread created");

        // Top-level (no parent) + labelled `tasks` so it lands in the Tasks tab.
        let threads = conversations::list_threads(ws.clone()).expect("list threads");
        let t = threads.iter().find(|t| t.id == id).expect("thread listed");
        assert!(
            t.parent_thread_id.is_none(),
            "session thread must be top-level"
        );
        assert!(
            t.labels.iter().any(|l| l == "tasks"),
            "must carry the tasks label"
        );

        // Seed user message carries the prompt + correlation metadata.
        let msgs = conversations::get_messages(ws, &id).expect("messages");
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].sender, "user");
        assert_eq!(msgs[0].content, "Do the thing");
    }

    #[test]
    fn append_final_writes_assistant_outcome() {
        let ws = temp_ws();
        let id = create_session_thread(ws.clone(), &card("X"), "run-2", "prompt").expect("thread");
        append_final(ws.clone(), &id, &Ok("All done.".to_string()));

        let msgs = conversations::get_messages(ws, &id).expect("messages");
        let last = msgs.last().expect("has messages");
        assert_eq!(last.sender, "assistant");
        assert_eq!(last.content, "All done.");
    }

    #[test]
    fn append_final_skips_empty_response() {
        let ws = temp_ws();
        let id = create_session_thread(ws.clone(), &card("X"), "run-3", "prompt").expect("thread");
        append_final(ws.clone(), &id, &Ok("   ".to_string()));

        let msgs = conversations::get_messages(ws, &id).expect("messages");
        assert_eq!(
            msgs.len(),
            1,
            "empty final response must not append a message"
        );
    }

    #[test]
    fn empty_title_falls_back_to_generic_label() {
        assert_eq!(session_title(&card("   ")), "Autonomous task");
        assert_eq!(session_title(&card("Real title")), "Real title");
    }
}
