//! Unit tests for [`super::TurnStateMirror`].

use super::*;
use crate::openhuman::agent::progress::AgentProgress;
use crate::openhuman::agent::task_board::{TaskBoard, TaskBoardCard, TaskCardStatus};
use tempfile::tempdir;

fn fresh(thread_id: &str) -> (tempfile::TempDir, TurnStateMirror) {
    let dir = tempdir().expect("tempdir");
    let store = TurnStateStore::new(dir.path().to_path_buf());
    let mirror = TurnStateMirror::new(store, thread_id, "req-1");
    (dir, mirror)
}

// ── Interrupted-partial → session transcript wiring (Task 1) ──────────

use crate::openhuman::agent::harness::session::transcript::{
    self, read_transcript, read_transcript_display, DisplayRecord, TranscriptMeta,
};
use crate::openhuman::agent::messages::ChatMessage;

fn seed_root_transcript(workspace: &std::path::Path, thread_id: &str) -> std::path::PathBuf {
    let stem = "100_orchestrator".to_string();
    let path = transcript::resolve_keyed_transcript_path(workspace, &stem).expect("resolve path");
    let meta = TranscriptMeta {
        agent_name: "orchestrator".into(),
        agent_id: None,
        agent_type: Some("root".into()),
        dispatcher: "native".into(),
        provider: None,
        model: None,
        created: "2026-07-21T00:00:00Z".into(),
        updated: "2026-07-21T00:00:00Z".into(),
        turn_count: 1,
        input_tokens: 0,
        output_tokens: 0,
        cached_input_tokens: 0,
        charged_amount_usd: 0.0,
        thread_id: Some(thread_id.to_string()),
        task_id: None,
    };
    transcript::write_transcript(&path, &[ChatMessage::user("hello there")], &meta, None)
        .expect("seed transcript");
    path
}

#[path = "mirror_tests_part_01_tests.rs"]
mod part_01_tests;
#[path = "mirror_tests_part_02_tests.rs"]
mod part_02_tests;
