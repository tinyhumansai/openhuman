//! Unit tests for [`super::TurnStateMirror`].

use super::*;
use crate::agent::progress::AgentProgress;
use tempfile::tempdir;

fn fresh(thread_id: &str) -> (tempfile::TempDir, TurnStateMirror) {
    let dir = tempdir().expect("tempdir");
    let store =
        std::sync::Arc::new(InMemoryTurnStates::default()) as std::sync::Arc<dyn TurnStates>;
    let mirror = TurnStateMirror::new(store, thread_id, "req-1");
    (dir, mirror)
}

#[path = "mirror_finish_and_subagent_args_tests.rs"]
mod finish_and_subagent_args_tests;
#[path = "mirror_observe_tests.rs"]
mod observe_tests;
