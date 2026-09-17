use std::{path::Path, sync::Arc};

use crate::agent::goose::{FileGooseCheckpointStore, GooseCheckpointStore};

const CHECKPOINT_DIR: &str = "goose_primary";

/// Workspace-owned durable primary checkpoint store. It survives process
/// restart while remaining outside the action directory exposed to tools.
pub(crate) fn primary_checkpoint_store(workspace_dir: &Path) -> Arc<FileGooseCheckpointStore> {
    Arc::new(FileGooseCheckpointStore::new(
        workspace_dir.join(CHECKPOINT_DIR),
    ))
}

pub(crate) async fn has_live_primary_checkpoint(workspace_dir: &Path, session_id: &str) -> bool {
    primary_checkpoint_store(workspace_dir)
        .load(session_id)
        .await
        .is_ok_and(|checkpoint| checkpoint.is_resumable())
}

pub(crate) fn clear_primary_checkpoint(workspace_dir: &Path, session_id: &str) {
    if let Err(error) = primary_checkpoint_store(workspace_dir).remove(session_id) {
        tracing::warn!(
            session_id,
            error = %error,
            "[primary-orchestration] failed to remove durable checkpoint"
        );
    }
}

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod runtime_tests;
