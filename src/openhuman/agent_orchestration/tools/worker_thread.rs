//! Shared helper for materialising a sub-agent's work as a persistent
//! conversation sub-thread.
//!
//! Both `spawn_worker_thread` (explicit) and `spawn_subagent` (inline)
//! back their run with a `worker-<uuid>` thread linked to the parent so the
//! delegation is reopenable from memory and rendered as a parent↔subagent
//! chat. The thread is created with `parent_thread_id` set (which hides it
//! from the main sidebar — it surfaces in the "Workers" tab and the
//! subagent drawer instead) and seeded with the delegation prompt as the
//! opening `user` message. The sub-agent runner then appends each turn and
//! tool result to the same thread via its `worker_thread_id` sink.

use std::path::PathBuf;

use serde_json::json;

use crate::openhuman::memory_conversations::{
    self as conversations, ConversationMessage, CreateConversationThread,
};

/// Create a worker sub-thread linked to `parent_thread_id` and seed it with
/// the delegation `prompt` as the opening user message. Returns the new
/// thread id, or an `Err` string if the thread store rejected the create.
///
/// The seed-message append is best-effort: a failure there is logged but
/// does not fail the call (the thread still exists and the runner will
/// append the sub-agent's turns).
pub(crate) fn create_worker_thread(
    workspace_dir: PathBuf,
    parent_thread_id: &str,
    agent_id: &str,
    title: &str,
    prompt: &str,
) -> Result<String, String> {
    let worker_thread_id = format!("worker-{}", uuid::Uuid::new_v4());
    let now = chrono::Utc::now().to_rfc3339();

    conversations::ensure_thread(
        workspace_dir.clone(),
        CreateConversationThread {
            id: worker_thread_id.clone(),
            title: title.to_string(),
            created_at: now.clone(),
            parent_thread_id: Some(parent_thread_id.to_string()),
            labels: Some(vec!["tasks".to_string()]),
            personality_id: None,
        },
    )?;

    tracing::info!(
        agent_id = %agent_id,
        worker_thread_id = %worker_thread_id,
        parent_thread_id = %parent_thread_id,
        "[worker_thread] created sub-thread for delegation"
    );

    if let Err(err) = conversations::append_message(
        workspace_dir,
        &worker_thread_id,
        ConversationMessage {
            id: format!("user:{}", uuid::Uuid::new_v4()),
            content: prompt.to_string(),
            message_type: "text".to_string(),
            extra_metadata: json!({ "scope": "worker_thread", "agent_id": agent_id }),
            sender: "user".to_string(),
            created_at: now,
        },
    ) {
        tracing::warn!(
            worker_thread_id = %worker_thread_id,
            error = %err,
            "[worker_thread] failed to seed delegation prompt (continuing)"
        );
    }

    Ok(worker_thread_id)
}

pub(crate) fn append_worker_user_message(
    workspace_dir: PathBuf,
    worker_thread_id: &str,
    agent_id: &str,
    task_id: &str,
    prompt: &str,
) -> Result<(), String> {
    conversations::append_message(
        workspace_dir,
        worker_thread_id,
        ConversationMessage {
            id: format!("user:{task_id}:{}", uuid::Uuid::new_v4()),
            content: prompt.to_string(),
            message_type: "text".to_string(),
            extra_metadata: json!({
                "scope": "worker_thread",
                "agent_id": agent_id,
                "task_id": task_id,
                "reused_worker": true,
            }),
            sender: "user".to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
        },
    )
    .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_child_thread_linked_to_parent_and_seeds_prompt() {
        let dir = std::env::temp_dir().join(format!("wt-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();

        let id = create_worker_thread(
            dir.clone(),
            "parent-thread-1",
            "researcher",
            "Q3",
            "Find Q3",
        )
        .expect("thread should be created");

        // The new thread is labelled `tasks` and linked to the parent so it
        // stays grouped with delegated task work in the chat sidebar.
        let threads = conversations::list_threads(dir.clone()).unwrap();
        let thread = threads
            .iter()
            .find(|t| t.id == id)
            .expect("thread persisted");
        assert_eq!(thread.parent_thread_id.as_deref(), Some("parent-thread-1"));
        assert!(thread.labels.contains(&"tasks".to_string()));

        // It opens with the delegation prompt as the user message, so the
        // drawer can render the parent↔subagent chat from memory on reopen.
        let messages = conversations::get_messages(dir.clone(), &id).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].sender, "user");
        assert_eq!(messages[0].content, "Find Q3");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn appends_follow_up_prompt_to_existing_worker_thread() {
        let dir = std::env::temp_dir().join(format!("wt-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let id = create_worker_thread(
            dir.clone(),
            "parent-thread-1",
            "researcher",
            "Initial",
            "Initial prompt",
        )
        .unwrap();

        append_worker_user_message(dir.clone(), &id, "researcher", "sub-2", "Follow-up prompt")
            .unwrap();

        let messages = conversations::get_messages(dir.clone(), &id).unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1].sender, "user");
        assert_eq!(messages[1].content, "Follow-up prompt");
        let _ = std::fs::remove_dir_all(dir);
    }
}
