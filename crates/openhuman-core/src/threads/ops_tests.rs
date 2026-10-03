//! Shape + validation tests for the pure, pre-IO helpers used by the
//! threads RPC surface. Every test here avoids disk, network, and
//! provider calls — they pin the behaviour of the branches that all of
//! the async `ops::*` entry points rely on.
use super::*;
// Re-imported here rather than through `ops`: `ops` itself no longer names
// these, so importing them there would be an unused import in a non-test build.
use crate::config::test_env::EnvVarGuard;
use crate::threads::store as conversations_store;
use crate::threads::turn_state::{ClearTurnStateRequest, GetTurnStateRequest};
use crate::threads::ThreadsError;
use serde_json::{json, Value};
use tinyagents_harness::title::{build_title_prompt, THREAD_TITLE_SYSTEM_PROMPT};
use tinyagents_session::turn_state::TurnState;

// ── thread_to_summary / message_to_record / record_to_message ─

fn sample_thread() -> ConversationThread {
    ConversationThread {
        id: "t-1".into(),
        title: "My thread".into(),
        chat_id: Some(42),
        is_active: true,
        message_count: 5,
        last_message_at: "2026-01-01T00:00:00Z".into(),
        created_at: "2026-01-01T00:00:00Z".into(),
        parent_thread_id: None,
        labels: vec!["general".to_string()],
        personality_id: None,
    }
}

fn sample_message() -> ConversationMessage {
    ConversationMessage {
        id: "m-1".into(),
        content: "hi".into(),
        message_type: "text".into(),
        extra_metadata: json!({"k": "v"}),
        sender: "user".into(),
        created_at: "2026-01-01T00:00:00Z".into(),
    }
}

async fn create_thread_with_title(_workspace: &tempfile::TempDir, thread_id: &str, title: &str) {
    let dir = crate::config::Config::load_or_init()
        .await
        .expect("load config")
        .workspace_dir;
    conversations_store::ensure_thread(
        dir,
        CreateConversationThread {
            id: thread_id.to_string(),
            title: title.to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            parent_thread_id: None,
            labels: None,
            personality_id: None,
        },
    )
    .expect("ensure thread");
}

#[path = "ops_conversion_tests.rs"]
mod conversion_tests;
#[path = "ops_title_and_cancellation_tests.rs"]
mod title_and_cancellation_tests;
