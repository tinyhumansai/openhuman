//! Shape + validation tests for the pure, pre-IO helpers used by the
//! threads RPC surface. Every test here avoids disk, network, and
//! provider calls — they pin the behaviour of the branches that all of
//! the async `ops::*` entry points rely on.
use super::*;
// Re-imported here rather than through `ops`: `ops` itself no longer names
// these, so importing them there would be an unused import in a non-test build.
use crate::openhuman::memory::conversations as conversations_store;
use crate::openhuman::threads::title::{build_title_prompt, THREAD_TITLE_SYSTEM_PROMPT};
use crate::openhuman::threads::turn_state::{
    self, ClearTurnStateRequest, GetTurnStateRequest, TurnState,
};
use crate::openhuman::threads::ThreadsError;
use serde_json::{json, Value};
use std::ffi::OsString;
use std::path::Path;

struct EnvVarGuard {
    key: &'static str,
    old: Option<OsString>,
}

impl EnvVarGuard {
    fn set_to_path(key: &'static str, value: &Path) -> Self {
        let old = std::env::var_os(key);
        std::env::set_var(key, value.as_os_str());
        Self { key, old }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.old {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

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
    let dir = crate::openhuman::config::Config::load_or_init()
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

#[path = "ops_tests_part_01_tests.rs"]
mod part_01_tests;
#[path = "ops_tests_part_02_tests.rs"]
mod part_02_tests;
