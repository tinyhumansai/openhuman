//! Importing a v1 memory store into the selected engine.
//!
//! The driver (batching, checkpoints, backoff, failed-item list, retry) is
//! `tinymemory_integrations::import::Importer`. This file is the host
//! adapter around it: the consent gate, the one-run-per-workspace registry,
//! the atomic state file `<workspace>/memory/import_state.json`, the tokio
//! clock, the scheduler pause, the credits resume policy, and what follows a
//! finished import (organizing the tree).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use tinymemory_integrations::import::{
    Begun, CheckpointStore, ImportError, ImportFile, Importer, Pacer,
};

use crate::config::Config;

use super::engine;
use super::error::{MemoryError, MemoryResult};
use super::types::{ImportCounts, ImportPhase, ImportScanView, ImportState};

/// Whether background work is paused right now. An automatic resume asks
/// before it starts and again before every batch it stores.
pub(crate) type PauseCheck = Arc<dyn Fn() -> bool + Send + Sync>;

/// Whether memory work that uploads on its own, with no user action (an
/// automatic import, or resuming one paused for credits), may run now.
pub(crate) type BillingCheck = Arc<dyn Fn(&Config) -> bool + Send + Sync>;

/// Imports running now, per workspace.
static RUNNING: LazyLock<Mutex<HashSet<PathBuf>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

/// Held while an import or a layout migration decides to start: each checks
/// the other is not going, then registers itself, under this one lock, so
/// neither can slip in between the other's check and its registration.
pub(crate) static START_GATE: Mutex<()> = Mutex::new(());

fn running(workspace_dir: &Path) -> bool {
    RUNNING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .contains(workspace_dir)
}

fn release(workspace_dir: &Path) {
    RUNNING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(workspace_dir);
}

/// The state file, written atomically (staged, synced, renamed): a crash
/// mid-write leaves the previous state, never a torn file that reads back as
/// a fresh start and re-sends everything.
pub(crate) struct FileCheckpointStore {
    workspace_dir: PathBuf,
}

const STATE: &str = "import_state";

impl FileCheckpointStore {
    pub(crate) fn new(workspace_dir: &Path) -> Self {
        Self {
            workspace_dir: workspace_dir.to_path_buf(),
        }
    }
}

impl CheckpointStore for FileCheckpointStore {
    fn load(&self) -> ImportFile {
        super::state_store::load(&self.workspace_dir, STATE)
            .unwrap_or_else(|error| {
                tracing::warn!(%error, "[memory:import] reading the checkpoint failed");
                None
            })
            .unwrap_or_default()
    }

    fn save(&self, file: &ImportFile) -> Result<(), String> {
        super::state_store::save(&self.workspace_dir, STATE, file)
            .map_err(|error| error.to_string())
    }
}

/// Backoff sleeps on the tokio clock.
struct TokioPacer;

#[async_trait::async_trait]
impl Pacer for TokioPacer {
    async fn sleep(&self, delay: Duration) {
        tokio::time::sleep(delay).await;
    }
}

fn read_file(workspace_dir: &Path) -> ImportFile {
    FileCheckpointStore::new(workspace_dir).load()
}

/// Whether the last import ended with its last batch not confirmed listed:
/// the layout migration then forgets by id, never erasing scopes whole, and
/// copies again before it finishes. Only an import that confirms clears it.
pub(crate) fn listed_unconfirmed(workspace_dir: &Path) -> bool {
    read_file(workspace_dir).listed_unconfirmed
}

fn import_error(error: &ImportError) -> MemoryError {
    MemoryError::invalid(error.to_string())
}

fn importer(
    config: &Config,
    engine: Arc<dyn tinymemory_api::MemoryEngine>,
    consent: bool,
    paused: Option<PauseCheck>,
) -> Arc<Importer> {
    let mut importer = Importer::new(
        config.workspace_dir.clone(),
        engine,
        Arc::new(FileCheckpointStore::new(&config.workspace_dir)),
        Arc::new(TokioPacer),
        Arc::new(move || consent),
    )
    .with_describe(Arc::new(|error| {
        MemoryError::from(error.clone()).to_string()
    }));
    if let Some(paused) = paused {
        importer = importer.with_pause(paused);
    }
    Arc::new(importer)
}

/// Counts what a legacy store at `workspace_dir` holds, or `None` when there
/// is no v1 store there. Blocking (SQLite, and memory-tree chunk files).
pub fn count_legacy(workspace_dir: &Path) -> Option<ImportCounts> {
    let counts = tinymemory_integrations::import::count_legacy(workspace_dir)?;
    Some(ImportCounts {
        documents: counts.documents,
        conversations: counts.conversations,
        learnings: counts.learnings,
    })
}

/// `memory_import_scan`.
pub async fn scan(config: &Config) -> MemoryResult<ImportScanView> {
    let workspace_dir = config.workspace_dir.clone();
    let counts = tokio::task::spawn_blocking(move || count_legacy(&workspace_dir))
        .await
        .map_err(|error| MemoryError::Engine(format!("import scan failed: {error}")))?;
    Ok(ImportScanView {
        found: counts.is_some(),
        counts,
    })
}

/// Whether an import is unfinished: running now, interrupted (it resumes on
/// its own), or paused for credits. The layout migration waits for it, so
/// old local memory lands before the tree is reorganised.
#[must_use]
pub fn in_progress(config: &Config) -> bool {
    // A live import counts from its reservation, before its first state
    // write (the scan of the old store comes first).
    let file = read_file(&config.workspace_dir);
    running(&config.workspace_dir)
        || file.state.phase == ImportPhase::Running
        || file.paused_for_credits
}

/// `memory_import_status`.
#[must_use]
pub fn status(config: &Config) -> ImportState {
    let mut state = read_file(&config.workspace_dir).state;
    if state.phase == ImportPhase::Running && !running(&config.workspace_dir) {
        state.phase = ImportPhase::Error;
        state.error = Some(state.error.unwrap_or_else(|| {
            "the import was interrupted; it resumes on its own within a few minutes, or resume \
             it now"
                .to_string()
        }));
    }
    state
}

/// The scheduler's pause, which includes being signed out.
pub(crate) fn scheduler_paused() -> bool {
    matches!(
        crate::platform::llm_gate::current_policy(),
        crate::platform::llm_gate::Policy::Paused { .. }
    )
}

/// Resumes an import the app quit in the middle of, if there is one, under
/// the scheduler's pause. Called from memory's background job.
pub async fn resume_interrupted(ctx: &crate::core::runtime::DispatchCtx, config: &Config) -> bool {
    // A credits pause resumes on its own only while memory work is free
    // (`billing::free_period_active`). The backend is asked only when such a
    // pause is waiting on it.
    let file = read_file(&config.workspace_dir);
    let free = file.state.phase == ImportPhase::Error
        && file.paused_for_credits
        && super::billing::free_period_active(config).await;
    resume_interrupted_with(
        ctx,
        config,
        Arc::new(scheduler_paused),
        Arc::new(move |_: &Config| free),
    )
    .await
}

/// Resumes an interrupted import unless `paused` says background work is
/// paused.
///
/// Only a `Running` state with no live run counts: the user consented when
/// it started. A paused-for-credits run resumes once `billing` allows. A
/// resumed run asks `paused` again before every batch. An automatic resume
/// that cannot start is persisted as stopped, so a failure that would recur
/// does not loop. Returns whether a run was started.
pub(crate) async fn resume_interrupted_with(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    paused: PauseCheck,
    billing: BillingCheck,
) -> bool {
    let file = read_file(&config.workspace_dir);
    let resumable = match file.state.phase {
        ImportPhase::Running => true,
        ImportPhase::Error => file.paused_for_credits && billing(config),
        _ => false,
    };
    if !resumable || running(&config.workspace_dir) {
        return false;
    }
    if paused() {
        tracing::debug!("[memory:import] background paused; interrupted import left for later");
        return false;
    }
    let retrying = file.state.phase == ImportPhase::Running && file.retrying;
    let resumed = if retrying {
        launch(ctx, config, Mode::ResumeRetry, true, Some(paused)).await
    } else {
        launch(ctx, config, Mode::Import, true, Some(paused)).await
    };
    match resumed {
        Ok(state) => {
            tracing::info!(
                imported = state.imported,
                total = state.total,
                "[memory:import] resuming an interrupted import"
            );
            true
        }
        Err(error) => {
            tracing::warn!(
                code = error.code(),
                "[memory:import] interrupted import could not resume; stopped"
            );
            let engine = engine::resolve(config).engine().map(|bound| bound.engine);
            // Without an engine the importer is only used for its store.
            if let Ok(engine) = engine {
                importer(config, engine, true, None).record_resume_failure(&error.to_string());
            } else {
                let store = FileCheckpointStore::new(&config.workspace_dir);
                let mut file = store.load();
                file.state.phase = ImportPhase::Error;
                file.state.error = Some(format!("the import could not resume: {error}"));
                file.paused_for_credits = false;
                if let Err(error) = store.save(&file) {
                    tracing::warn!(%error, "[memory:import] writing import state failed");
                }
            }
            false
        }
    }
}

/// `memory_import_start`: requires `consent`, memory on, and a legacy store.
pub async fn start(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    consent: bool,
) -> MemoryResult<ImportState> {
    launch(ctx, config, Mode::Import, consent, None).await
}

/// `memory_import_retry_failed`: stores again the items a finished import
/// skipped because the engine refused them.
pub async fn retry_failed(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
) -> MemoryResult<ImportState> {
    launch(ctx, config, Mode::Retry, true, None).await
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Import,
    Retry,
    ResumeRetry,
}

/// Claims the workspace, begins the run on the module's driver and spawns it.
async fn launch(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    mode: Mode,
    consent: bool,
    paused: Option<PauseCheck>,
) -> MemoryResult<ImportState> {
    if mode == Mode::Import && !consent {
        return Err(import_error(&ImportError::ConsentRequired));
    }
    let bound = engine::resolve(config).engine()?;
    let workspace_dir = config.workspace_dir.clone();
    let claimed = {
        let _gate = START_GATE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // Items written to the legacy tree while it is being moved would be
        // left behind there.
        if super::layout_migration::service::is_running(config) {
            return Err(MemoryError::invalid(if mode == Mode::Import {
                "memory is being organized; import once that finishes"
            } else {
                "memory is being organized; retry once that finishes"
            }));
        }
        RUNNING
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(workspace_dir.clone())
    };
    if !claimed {
        return Ok(status(config));
    }
    let driver = importer(config, bound.engine, consent, paused);
    let begin = {
        let driver = driver.clone();
        ctx.spawn_blocking(move || match mode {
            Mode::Import => driver.begin(),
            Mode::Retry => driver.begin_retry(),
            Mode::ResumeRetry => Ok(driver.begin_retry_resume()),
        })
        .await
    };
    let begun: Begun = match begin {
        Ok(Ok(begun)) => begun,
        Ok(Err(error)) => {
            release(&workspace_dir);
            return Err(import_error(&error));
        }
        Err(error) => {
            release(&workspace_dir);
            return Err(MemoryError::Engine(format!("import start failed: {error}")));
        }
    };
    let state = begun.state.clone();
    tracing::info!(
        total = state.total,
        retry = mode != Mode::Import,
        "[memory:import] run started"
    );
    let config = config.clone();
    let task_ctx = ctx.clone();
    ctx.spawn(async move {
        if mode == Mode::Import {
            driver.run(begun).await;
        } else {
            driver.run_retry(begun).await;
        }
        release(&workspace_dir);
        // Import first, then organize: the move starts here, on whatever
        // screen the user is, once the import is done.
        if mode == Mode::Import && read_file(&workspace_dir).state.phase == ImportPhase::Done {
            let started = super::layout_migration::start(
                &task_ctx,
                config.clone(),
                Arc::new(super::layout_migration::AppHost::new(config)),
                // Migration is free: organizing follows a finished import
                // straight away. A shared self-hosted tree still waits for
                // the takeover consent.
                super::layout_migration::Trigger::Manual { takeover: false },
                Arc::new(scheduler_paused),
            );
            tracing::info!(started, "[memory:import] import done; organizing next");
        }
    });
    Ok(state)
}

#[cfg(test)]
#[path = "import_tests.rs"]
mod tests;
