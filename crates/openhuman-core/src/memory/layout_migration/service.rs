//! The migration as a service: what the RPCs and the background job call.
//!
//! - [`scan`]: whether there is anything to move (the UI shows nothing when
//!   there is not).
//! - [`start`]: runs the migration in the background; at most one run per
//!   workspace at a time.
//! - [`status`]: its progress, with a run the app quit in the middle of
//!   reported as interrupted.
//! - [`retry`]: puts the items that could not be moved back in line.
//! - [`tick`]: memory's background job, every few minutes. It starts or
//!   resumes an automatic run, which itself waits for a free period (and,
//!   on a shared legacy tree, for the user's consent).
//!
//! The state machine is the module's; this file owns the executor side: the
//! spawned task, the run slot per workspace and the import gate.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex, PoisonError};

use tinymemory_tools::layout_migration as lm;

use super::app_host::{from_module, AppHost};
use super::{LayoutHost, MigrationState, MigrationStatus, ScanView, Trigger};
use crate::config::Config;
use crate::memory::error::MemoryResult;

/// Workspaces with a migration running now.
static RUNNING: LazyLock<Mutex<HashSet<PathBuf>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

/// `memory_migration_start` params.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct StartParams {
    /// The user's consent to take a legacy tree other accounts may share.
    #[serde(default)]
    pub takeover: bool,
}

/// Whether `host`'s account has legacy memory to move.
///
/// # Errors
///
/// When the state cannot be read, the engines cannot be bound, or the legacy
/// tree cannot be read.
pub async fn scan(host: &dyn LayoutHost) -> MemoryResult<ScanView> {
    lm::scan(host).await.map_err(from_module)
}

/// The migration's progress for `config`'s workspace.
///
/// # Errors
///
/// When the state file cannot be read.
pub fn status(config: &Config) -> MemoryResult<MigrationStatus> {
    lm::status(&AppHost::new(config.clone()), is_running(config)).map_err(from_module)
}

pub(crate) fn is_running(config: &Config) -> bool {
    RUNNING
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .contains(&config.workspace_dir)
}

/// Releases the workspace's run slot when the run ends, however it ends.
struct RunGuard(PathBuf);

impl Drop for RunGuard {
    fn drop(&mut self) {
        RUNNING
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.0);
    }
}

/// Starts a run in the background. Returns `false`, starting nothing, when
/// one is already running for this workspace, or while an import of old
/// local memory is unfinished (import first, then reorganise).
pub fn start(
    ctx: &crate::core::runtime::DispatchCtx,
    config: Config,
    host: Arc<dyn LayoutHost>,
    trigger: Trigger,
    paused: Arc<dyn Fn() -> bool + Send + Sync>,
) -> bool {
    let _gate = crate::memory::import::START_GATE
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if crate::memory::import::in_progress(&config) {
        tracing::info!("[memory:layout_migration] waiting for the import to finish");
        return false;
    }
    let workspace = config.workspace_dir.clone();
    if !RUNNING
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(workspace.clone())
    {
        return false;
    }
    let guard = RunGuard(workspace);
    ctx.spawn(async move {
        let _guard = guard;
        let outcome = lm::run(host.as_ref(), trigger, || {
            let paused = paused();
            async move { paused }
        })
        .await;
        match outcome {
            Ok(outcome) => {
                tracing::info!(?outcome, "[memory:layout_migration] run ended");
            }
            Err(error) => {
                tracing::warn!(code = error.code(), "[memory:layout_migration] run failed");
                lm::record_failure(host.as_ref(), &error);
            }
        }
    });
    true
}

/// Puts the items that could not be moved back in line: the next run copies
/// again from the start (what was copied replays) and cleans up again.
///
/// # Errors
///
/// When the state cannot be read or saved, or a run is going now.
pub fn retry(config: &Config) -> MemoryResult<MigrationState> {
    lm::retry(&AppHost::new(config.clone()), is_running(config)).map_err(from_module)
}

/// Memory's background job: starts or resumes an automatic run unless the
/// migration is done or already running. Returns whether one started.
pub fn tick(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    host: Arc<dyn LayoutHost>,
    paused: Arc<dyn Fn() -> bool + Send + Sync>,
) -> bool {
    match lm::is_finished(host.as_ref()) {
        Ok(true) => false,
        Ok(false) if paused() || is_running(config) => false,
        Ok(false) => start(ctx, config.clone(), host, Trigger::Auto, paused),
        Err(error) => {
            tracing::warn!(
                code = error.code(),
                "[memory:layout_migration] state unreadable"
            );
            false
        }
    }
}

#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;
