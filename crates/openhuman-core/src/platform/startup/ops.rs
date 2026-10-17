use std::path::Path;

/// Run one-shot workspace migrations needed at process startup.
///
/// Failures are logged and do not abort startup. Individual migration helpers
/// remain responsible for their own idempotency markers.
pub fn run_workspace_migrations(ctx: &crate::core::runtime::DispatchCtx, workspace_dir: &Path) {
    match crate::threads::migrate_welcome_agent_artifacts(ctx, workspace_dir) {
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

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
