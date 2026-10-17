//! The persisted store behind the session todo list.
//!
//! Todos are session state, the way Claude Code and Codex keep them: one list
//! per conversation thread. Persisted in the
//! agent's key-value store — the one thread goals use
//! (`agent::goals::store`) — so a list survives a core restart instead of resetting with the old
//! process-global `InMemoryStore`. The transcript still records every list
//! the model wrote, and the frontend renders the latest one from the turn's
//! `todo` tool call.

use std::path::Path;
use std::sync::Arc;

use tinyagents_harness::store::Store;

/// The store every session's list lives in, keyed by session/thread id: the
/// current agent's key-value store in the storage backend. Resolved per call
/// so it always reflects the caller's current workspace and tenant.
pub fn session_todos_store(workspace_dir: &Path) -> Arc<dyn Store> {
    crate::agent::session_store::stores_or_ephemeral(workspace_dir).kv
}

/// Synthetic key for a tool call that has no session at all (a bare
/// `Tool::execute` in a test).
pub const SCRATCH_SESSION_ID: &str = "_scratch_";
