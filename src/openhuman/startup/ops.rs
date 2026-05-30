use std::path::Path;

/// Run one-shot workspace migrations needed at process startup.
///
/// Failures are logged and do not abort startup. Individual migration helpers
/// remain responsible for their own idempotency markers.
pub fn run_workspace_migrations(workspace_dir: &Path) {
    match crate::openhuman::agent::harness::session::migrate_session_layout_if_needed(workspace_dir)
    {
        Ok(outcome) if outcome.already_done => {
            log::debug!("[runtime] session_layout migration already applied");
        }
        Ok(outcome) => {
            log::info!(
                "[runtime] session_layout migration applied: jsonl_moved={} md_moved={} pruned_dirs={} warnings={}",
                outcome.jsonl_moved,
                outcome.md_moved,
                outcome.legacy_dirs_pruned,
                outcome.warnings.len(),
            );
            for warning in &outcome.warnings {
                log::warn!("[runtime] session_layout migration warning: {warning}");
            }
        }
        Err(err) => {
            log::warn!(
                "[runtime] session_layout migration failed: {err} — \
                 falling back to in-place legacy reads"
            );
        }
    }

    match crate::openhuman::threads::migrate_welcome_agent_artifacts(workspace_dir) {
        Ok(result) if result.already_done => {
            log::debug!("[migration::welcome-to-orchestrator] already applied");
        }
        Ok(result)
            if result.threads_updated == 0
                && result.transcripts_updated == 0
                && result.transcript_files_renamed == 0
                && result.markdown_files_renamed == 0 =>
        {
            log::debug!("[migration::welcome-to-orchestrator] no artifacts to update");
        }
        Ok(result) => {
            log::info!(
                "[migration::welcome-to-orchestrator] threads_updated={} transcripts_updated={} transcript_files_renamed={} markdown_files_renamed={}",
                result.threads_updated,
                result.transcripts_updated,
                result.transcript_files_renamed,
                result.markdown_files_renamed
            );
        }
        Err(err) => {
            log::warn!("[migration::welcome-to-orchestrator] migration failed: {err}");
        }
    }
}
