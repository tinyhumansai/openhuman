//! Registry of in-flight async sub-agents that can be **steered** mid-run.
//!
//! `spawn_async_subagent` runs a child as a detached `tokio` task. On its own
//! that task is opaque: the parent gets a `task_id` back but has no channel into
//! the running loop and no way to collect the result inline. This registry
//! closes both gaps.
//!
//! Each running async sub-agent registers, keyed by its `task_id`, with:
//! - an `Arc<RunQueue>` — the same steering channel the steering forwarder in
//!   `run_turn_via_tinyagents_shared` drains mid-turn, so `steer_subagent` can
//!   inject a message when no crate-native steering handle is registered;
//! - a TinyAgents `SteeringHandle` in the process-local
//!   `SteeringRegistry` while the child TinyAgents run is active, so
//!   steer/collect controls can deliver directly to the crate queue;
//! - a `watch::Receiver<SubagentStatus>` — so `wait_subagent` can block until the
//!   child reaches a terminal status;
//! - an `AbortHandle` — used by `subagent_cancel`/`close_subagent` paths to stop
//!   detached work.
//!
//! Ownership is enforced: only the spawning parent (matched by `parent_session`)
//! may steer or wait on a given sub-agent. Terminal entries are pruned on `wait`,
//! and swept on `register` only once the table passes a soft cap, so it can't
//! grow unbounded if a parent never waits (the Codex "spawn-slot leak" failure
//! mode — openai/codex#18335).
//!
//! ## Typed lifecycle ledger (issue #4249)
//!
//! Alongside the executor plumbing (abort handle + steering queue + watch
//! status), every detached sub-agent is also recorded in a process-wide
//! [`tinyagents` orchestration `TaskStore`](crate::openhuman::tinyagents::orchestration)
//! as an `OrchestrationTaskKind::SubAgent` task. `register` inserts it
//! (`Pending` → `Running`) and spawns a watcher that mirrors the child's
//! terminal status into the store (`Completed`/`Failed`/`Awaiting`); the cancel
//! paths record `Cancelled`. This gives a typed, queryable lifecycle
//! (`task_records`) without disturbing the watch/abort/steer machinery the store
//! does not cover.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use tokio::sync::watch;
use tokio::task::AbortHandle;

use crate::openhuman::agent::harness::run_queue::{QueueMode, QueuedMessage, RunQueue};
use crate::openhuman::tinyagents::orchestration::{
    shared_steering_registry, InMemoryTaskStore, JsonlTaskStore, OrchestrationTaskFilter,
    OrchestrationTaskKind, OrchestrationTaskRecord, OrchestrationTaskResult, OrchestrationTaskSpec,
    OrchestrationTaskStatus, SteeringCommand, SteeringCommandKind, TaskStore,
};
use tinyagents::harness::ids::TaskId;
use tinyagents::harness::message::Message as TaMessage;
use tinyagents::CancellationToken;

enum DetachedTaskStore {
    Durable(JsonlTaskStore),
    Memory(InMemoryTaskStore),
}

impl TaskStore for DetachedTaskStore {
    fn insert(&self, spec: OrchestrationTaskSpec) -> tinyagents::Result<OrchestrationTaskRecord> {
        match self {
            Self::Durable(store) => store.insert(spec),
            Self::Memory(store) => store.insert(spec),
        }
    }

    fn get(&self, task_id: &TaskId) -> Option<OrchestrationTaskRecord> {
        match self {
            Self::Durable(store) => store.get(task_id),
            Self::Memory(store) => store.get(task_id),
        }
    }

    fn list(&self, filter: OrchestrationTaskFilter) -> Vec<OrchestrationTaskRecord> {
        match self {
            Self::Durable(store) => store.list(filter),
            Self::Memory(store) => store.list(filter),
        }
    }

    fn history(&self, task_id: &TaskId) -> Vec<OrchestrationTaskRecord> {
        match self {
            Self::Durable(store) => store.history(task_id),
            Self::Memory(store) => store.history(task_id),
        }
    }

    fn mark_running(&self, task_id: &TaskId) -> tinyagents::Result<OrchestrationTaskRecord> {
        match self {
            Self::Durable(store) => store.mark_running(task_id),
            Self::Memory(store) => store.mark_running(task_id),
        }
    }

    fn mark_awaiting(&self, task_id: &TaskId) -> tinyagents::Result<OrchestrationTaskRecord> {
        match self {
            Self::Durable(store) => store.mark_awaiting(task_id),
            Self::Memory(store) => store.mark_awaiting(task_id),
        }
    }

    fn complete(
        &self,
        task_id: &TaskId,
        result: OrchestrationTaskResult,
    ) -> tinyagents::Result<OrchestrationTaskRecord> {
        match self {
            Self::Durable(store) => store.complete(task_id, result),
            Self::Memory(store) => store.complete(task_id, result),
        }
    }

    fn fail(&self, task_id: &TaskId, error: String) -> tinyagents::Result<OrchestrationTaskRecord> {
        match self {
            Self::Durable(store) => store.fail(task_id, error),
            Self::Memory(store) => store.fail(task_id, error),
        }
    }

    fn timeout(
        &self,
        task_id: &TaskId,
        error: String,
    ) -> tinyagents::Result<OrchestrationTaskRecord> {
        match self {
            Self::Durable(store) => store.timeout(task_id, error),
            Self::Memory(store) => store.timeout(task_id, error),
        }
    }

    fn request_cancel(
        &self,
        task_id: &TaskId,
    ) -> tinyagents::Result<crate::openhuman::tinyagents::orchestration::OrchestrationControlOutcome>
    {
        match self {
            Self::Durable(store) => store.request_cancel(task_id),
            Self::Memory(store) => store.request_cancel(task_id),
        }
    }

    fn mark_cancelled(&self, task_id: &TaskId) -> tinyagents::Result<OrchestrationTaskRecord> {
        match self {
            Self::Durable(store) => store.mark_cancelled(task_id),
            Self::Memory(store) => store.mark_cancelled(task_id),
        }
    }

    fn kill(
        &self,
        task_id: &TaskId,
    ) -> tinyagents::Result<crate::openhuman::tinyagents::orchestration::OrchestrationControlOutcome>
    {
        match self {
            Self::Durable(store) => store.kill(task_id),
            Self::Memory(store) => store.kill(task_id),
        }
    }

    fn set_timeout_ms(
        &self,
        task_id: &TaskId,
        timeout_ms: u64,
    ) -> tinyagents::Result<OrchestrationTaskRecord> {
        match self {
            Self::Durable(store) => store.set_timeout_ms(task_id, timeout_ms),
            Self::Memory(store) => store.set_timeout_ms(task_id, timeout_ms),
        }
    }
}

static TASK_STORES: OnceLock<Mutex<HashMap<PathBuf, Arc<DetachedTaskStore>>>> = OnceLock::new();

fn task_stores() -> &'static Mutex<HashMap<PathBuf, Arc<DetachedTaskStore>>> {
    TASK_STORES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn task_store_path(workspace_dir: &Path) -> PathBuf {
    workspace_dir
        .join(".openhuman")
        .join("orchestration_tasks.jsonl")
}

#[cfg(test)]
fn default_task_store_workspace() -> PathBuf {
    crate::openhuman::config::default_root_openhuman_dir()
        .map(|root| root.join("workspace"))
        .unwrap_or_else(|_| PathBuf::from(".openhuman").join("workspace"))
}

fn open_task_store(workspace_dir: &Path) -> DetachedTaskStore {
    let path = task_store_path(workspace_dir);
    if let Some(parent) = path.parent() {
        if let Err(err) = std::fs::create_dir_all(parent) {
            log::warn!(
                "[running_subagents] failed to create task store dir {}; falling back to memory: {}",
                parent.display(),
                err
            );
            return DetachedTaskStore::Memory(InMemoryTaskStore::new());
        }
    }

    match JsonlTaskStore::open(&path) {
        Ok(store) => {
            log::debug!(
                "[running_subagents] opened durable task store at {}",
                path.display()
            );
            DetachedTaskStore::Durable(store)
        }
        Err(err) => {
            log::warn!(
                "[running_subagents] failed to open durable task store {}; falling back to memory: {}",
                path.display(),
                err
            );
            DetachedTaskStore::Memory(InMemoryTaskStore::new())
        }
    }
}

/// Process-wide typed lifecycle ledger for detached sub-agents (issue #4249).
///
/// The first spawn opens a durable JSONL store under that workspace. Calls that
/// need a view before any spawn use the default internal workspace location.
fn task_store_for_workspace(workspace_dir: &Path) -> Arc<DetachedTaskStore> {
    let key = workspace_dir.to_path_buf();
    let mut stores = task_stores()
        .lock()
        .expect("running_subagents task store mutex poisoned");
    stores
        .entry(key.clone())
        .or_insert_with(|| Arc::new(open_task_store(&key)))
        .clone()
}

#[cfg(test)]
fn task_store() -> Arc<DetachedTaskStore> {
    let workspace = default_task_store_workspace();
    task_store_for_workspace(&workspace)
}

/// Record a freshly-spawned sub-agent in the store (`Pending` → `Running`).
/// Insert errors (e.g. a re-used task id across tests) are intentionally ignored.
fn record_spawned(
    task_id: &str,
    agent_id: &str,
    parent_session: &str,
    session_parent_prefix: Option<&str>,
    subagent_session_id: Option<&str>,
    workspace_dir: &Path,
    parent_thread_id: Option<&str>,
) {
    let store = task_store_for_workspace(workspace_dir);
    let root_run_id = session_parent_prefix
        .and_then(|prefix| prefix.split("__").next())
        .filter(|root| !root.is_empty())
        .unwrap_or(parent_session);
    let mut spec = OrchestrationTaskSpec::new(
        task_id.to_string(),
        OrchestrationTaskKind::SubAgent {
            agent: agent_id.to_string(),
        },
    )
    .with_lineage(parent_session.to_string(), root_run_id.to_string())
    .with_timeout_ms(DETACHED_LEDGER_TIMEOUT_MS)
    .with_metadata("parentSession", parent_session.to_string())
    .with_metadata("rootSession", root_run_id.to_string())
    .with_metadata(
        "defaultWaitTimeoutMs",
        DETACHED_LEDGER_TIMEOUT_MS.to_string(),
    )
    .with_metadata("workspaceDir", workspace_dir.display().to_string());
    if let Some(session_parent_prefix) = session_parent_prefix {
        spec = spec.with_metadata("sessionParentPrefix", session_parent_prefix.to_string());
    }
    if let Some(parent_thread_id) = parent_thread_id {
        spec = spec
            .with_thread(parent_thread_id.to_string())
            .with_metadata("parentThreadId", parent_thread_id.to_string());
    }
    if let Some(subagent_session_id) = subagent_session_id {
        spec = spec.with_metadata("subagentSessionId", subagent_session_id.to_string());
    }
    let _ = store.insert(spec);
    let _ = store.mark_running(&TaskId::new(task_id));
}

/// Mirror a child's published [`SubagentStatus`] into the typed store. Transition
/// errors (already terminal / cancelled) are ignored — first writer wins.
fn record_status(workspace_dir: &Path, task_id: &str, status: &SubagentStatus) {
    let store = task_store_for_workspace(workspace_dir);
    let id = TaskId::new(task_id);
    log::debug!(
        "[running_subagents] recording task status task_id={} workspace_dir={} terminal={}",
        task_id,
        workspace_dir.display(),
        status.is_terminal()
    );
    match status {
        SubagentStatus::Completed { output, .. } => {
            let _ = store.complete(&id, OrchestrationTaskResult::text(output.clone()));
        }
        SubagentStatus::Failed { error } => {
            let _ = store.fail(&id, error.clone());
        }
        SubagentStatus::AwaitingUser { .. } => {
            let _ = store.mark_awaiting(&id);
        }
        SubagentStatus::Running => {}
    }
}

/// Record a cancellation (`CancelRequested` → `Cancelled`) for `task_id`.
fn record_cancelled(workspace_dir: &Path, task_id: &str) {
    let store = task_store_for_workspace(workspace_dir);
    let id = TaskId::new(task_id);
    log::debug!(
        "[running_subagents] recording task cancellation task_id={} workspace_dir={}",
        task_id,
        workspace_dir.display()
    );
    let _ = store.request_cancel(&id);
    let _ = store.mark_cancelled(&id);
}

fn list_task_records(workspace_dir: &Path) -> Vec<OrchestrationTaskRecord> {
    let store = task_store_for_workspace(workspace_dir);
    store.list(OrchestrationTaskFilter::default().with_kind("sub_agent"))
}

/// Restart/resume reconciliation for detached sub-agents (issue #4249 / 07.2
/// steps 2 & 4).
///
/// A detached sub-agent runs as a `tokio` task owned by the process that spawned
/// it. When the core restarts, that task — and its live [`AbortHandle`] /
/// [`CancellationToken`] — is gone, but the durable [`JsonlTaskStore`] still
/// holds a non-terminal (`Pending`/`Running`/`Awaiting`/`CancelRequested`)
/// record for it. Such a record is **orphaned**: there is no live executor to
/// re-attach to (OpenHuman spawns child processes, so an in-flight run from a
/// dead parent cannot be resumed), and the run-ledger finalizer never observed a
/// terminal event, so it would otherwise render as a perpetual "running" entry.
///
/// This scans the workspace-scoped store for those orphans and reconciles each
/// to a terminal state — `Cancelled` if a cancel had been requested, otherwise
/// `Failed` with an "orphaned by restart" reason — then emits the existing typed
/// terminal lifecycle event ([`subagent_events::publish_subagent_failed`]) so the
/// run ledger finalizes. Best-effort and non-fatal: per-task transition errors
/// (e.g. a record that raced to terminal) are logged and skipped, and a
/// store-open failure simply reconciles nothing. Returns the count reconciled.
pub(crate) fn reconcile_orphaned_tasks_on_boot(workspace_dir: &Path) -> usize {
    let store = task_store_for_workspace(workspace_dir);
    let orphans: Vec<OrchestrationTaskRecord> = store
        .list(OrchestrationTaskFilter::default().with_kind("sub_agent"))
        .into_iter()
        .filter(|record| record.status.is_live())
        .collect();
    if orphans.is_empty() {
        log::debug!(
            "[taskstore] reconcile found no orphaned sub-agent tasks workspace_dir={}",
            workspace_dir.display()
        );
        return 0;
    }

    let mut reconciled = 0usize;
    for record in orphans {
        let task_id = record.spec.task_id.as_str().to_string();
        let id = TaskId::new(task_id.as_str());
        let prior = task_status_label(record.status);
        let reason = format!("sub-agent orphaned by core restart (was `{prior}`)");
        log::debug!(
            "[orchestrator] reconciling orphaned sub-agent task_id={} prior_status={} workspace_dir={}",
            task_id,
            prior,
            workspace_dir.display()
        );
        // A cancel-requested orphan settles as Cancelled; every other live state
        // settles as Failed (its driver died without a terminal event).
        let outcome = match record.status {
            OrchestrationTaskStatus::CancelRequested => store.mark_cancelled(&id).map(|_| ()),
            _ => store.fail(&id, reason.clone()).map(|_| ()),
        };
        match outcome {
            Ok(()) => {
                reconciled += 1;
                let parent_session = record_parent_session(&record)
                    .unwrap_or_default()
                    .to_string();
                let agent_id = record_agent_id(&record);
                // Reuse the 05.2 typed terminal lifecycle helper so the run
                // ledger finalizes exactly as it would for a live failure.
                super::subagent_events::publish_subagent_failed(
                    parent_session,
                    task_id.clone(),
                    agent_id,
                    reason.clone(),
                );
                log::info!(
                    "[orchestrator] reconciled orphaned sub-agent task_id={} prior_status={} -> terminal",
                    task_id,
                    prior
                );
            }
            Err(err) => {
                log::warn!(
                    "[taskstore] failed to reconcile orphaned sub-agent task_id={} prior_status={}: {}",
                    task_id,
                    prior,
                    err
                );
            }
        }
    }
    log::info!(
        "[taskstore] reconciled {reconciled} orphaned sub-agent task(s) on boot workspace_dir={}",
        workspace_dir.display()
    );
    reconciled
}

fn record_parent_session(record: &OrchestrationTaskRecord) -> Option<&str> {
    record
        .spec
        .metadata
        .get("parentSession")
        .map(String::as_str)
}

fn record_subagent_session_id(record: &OrchestrationTaskRecord) -> Option<&str> {
    record
        .spec
        .metadata
        .get("subagentSessionId")
        .map(String::as_str)
}

fn record_agent_id(record: &OrchestrationTaskRecord) -> String {
    match &record.spec.kind {
        OrchestrationTaskKind::SubAgent { agent } => agent.clone(),
        _ => "subagent".to_string(),
    }
}

pub(crate) fn task_record_for_task_in_workspace(
    workspace_dir: &Path,
    task_id: &str,
    parent_session: &str,
) -> Result<OrchestrationTaskRecord, WaitError> {
    let id = TaskId::new(task_id);
    let Some(record) = task_store_for_workspace(workspace_dir).get(&id) else {
        return Err(WaitError::Unknown);
    };
    if !matches!(record.spec.kind, OrchestrationTaskKind::SubAgent { .. }) {
        return Err(WaitError::Unknown);
    }
    if record_parent_session(&record) != Some(parent_session) {
        return Err(WaitError::NotOwned);
    }
    Ok(record)
}

fn record_to_status(record: OrchestrationTaskRecord) -> WaitOutcome {
    match record.status {
        OrchestrationTaskStatus::Completed => {
            let output = record
                .result
                .and_then(|result| {
                    result
                        .text
                        .or_else(|| result.output.map(|output| output.to_string()))
                })
                .unwrap_or_default();
            WaitOutcome::Terminal(SubagentStatus::Completed {
                output,
                iterations: 0,
            })
        }
        OrchestrationTaskStatus::Awaiting => WaitOutcome::Terminal(SubagentStatus::AwaitingUser {
            question: record.error.unwrap_or_else(|| {
                "sub-agent is awaiting user input; no clarification text was available from the durable task store".to_string()
            }),
        }),
        OrchestrationTaskStatus::Failed
        | OrchestrationTaskStatus::TimedOut
        | OrchestrationTaskStatus::Abandoned => WaitOutcome::Terminal(SubagentStatus::Failed {
            error: record.error.unwrap_or_else(|| {
                format!(
                    "sub-agent reached durable task status `{}`",
                    task_status_label(record.status)
                )
            }),
        }),
        OrchestrationTaskStatus::Cancelled => WaitOutcome::Terminal(SubagentStatus::Failed {
            error: "sub-agent was cancelled".to_string(),
        }),
        OrchestrationTaskStatus::Pending
        | OrchestrationTaskStatus::Running
        | OrchestrationTaskStatus::CancelRequested => WaitOutcome::TimedOut(SubagentStatus::Running),
    }
}

fn task_status_label(status: OrchestrationTaskStatus) -> &'static str {
    match status {
        OrchestrationTaskStatus::Pending => "pending",
        OrchestrationTaskStatus::Running => "running",
        OrchestrationTaskStatus::Awaiting => "awaiting",
        OrchestrationTaskStatus::Completed => "completed",
        OrchestrationTaskStatus::Failed => "failed",
        OrchestrationTaskStatus::CancelRequested => "cancel_requested",
        OrchestrationTaskStatus::Cancelled => "cancelled",
        OrchestrationTaskStatus::TimedOut => "timed_out",
        OrchestrationTaskStatus::Abandoned => "abandoned",
    }
}

/// Snapshot the typed lifecycle records, optionally scoped to a `parent_session`.
#[cfg(test)]
fn task_records(parent_session: Option<&str>) -> Vec<OrchestrationTaskRecord> {
    let _ = task_store();
    let stores: Vec<Arc<DetachedTaskStore>> = task_stores()
        .lock()
        .expect("running_subagents task store mutex poisoned")
        .values()
        .cloned()
        .collect();
    let all: Vec<OrchestrationTaskRecord> = stores
        .into_iter()
        .flat_map(|store| store.list(OrchestrationTaskFilter::default()))
        .collect();
    log::trace!(
        "[running_subagents] task_records loaded records={} parent_session_filter={:?}",
        all.len(),
        parent_session
    );
    match parent_session {
        Some(ps) => all
            .into_iter()
            .filter(|r| r.spec.metadata.get("parentSession").map(String::as_str) == Some(ps))
            .collect(),
        None => all,
    }
}

/// Terminal/transient state of a running async sub-agent, published by the
/// spawner's background task and observed by `wait_subagent`.
#[derive(Debug, Clone)]
pub(crate) enum SubagentStatus {
    /// Still executing its inner tool-call loop.
    Running,
    /// Finished normally with a final response.
    Completed { output: String, iterations: usize },
    /// Paused on `ask_user_clarification`; resume via `continue_subagent`.
    AwaitingUser { question: String },
    /// The run errored out.
    Failed { error: String },
}

impl SubagentStatus {
    fn is_terminal(&self) -> bool {
        !matches!(self, SubagentStatus::Running)
    }
}

struct RunningSubagentEntry {
    agent_id: String,
    parent_session: String,
    subagent_session_id: Option<String>,
    workspace_dir: PathBuf,
    /// Parent chat thread that spawned this sub-agent, captured at registration.
    /// `None` for a headless spawn with no originating thread. Used to abort the
    /// sub-agent when its parent thread is deleted (see [`cancel_for_thread`]).
    parent_thread_id: Option<String>,
    run_queue: Arc<RunQueue>,
    abort: AbortHandle,
    /// Cooperative-cancellation handle held **alongside** the hard-kill
    /// [`AbortHandle`] (issue #4249 / 07.2 step 2). The cancel/kill paths flip
    /// this token *before* aborting so a run that has opted into cooperative
    /// cancellation (a crate `CancellationToken` threaded into its `RunContext`)
    /// can unwind cleanly at its next safe checkpoint; the abort remains the
    /// executor-detail hard stop for runs that have not. Latching + cheap to
    /// clone, so cancelling it is always safe/idempotent.
    cancel: CancellationToken,
    status: watch::Receiver<SubagentStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SubagentResumeRef {
    pub(crate) task_id: String,
    pub(crate) agent_id: String,
    pub(crate) subagent_session_id: Option<String>,
}

/// Soft cap on registry size. Terminal entries are only swept when the table
/// grows past this, so the common case (a handful of live sub-agents) never
/// evicts a still-uncollected terminal result out from under a `wait`/`steer`.
const REGISTRY_SOFT_CAP: usize = 256;
/// Metadata-only timeout mirrored into the TinyAgents task ledger. It matches
/// `wait_subagent`'s default wait window; execution remains governed by the
/// existing detached task and wait-tool paths.
const DETACHED_LEDGER_TIMEOUT_MS: u64 = 120_000;

static REGISTRY: OnceLock<Mutex<HashMap<String, RunningSubagentEntry>>> = OnceLock::new();

fn registry() -> &'static Mutex<HashMap<String, RunningSubagentEntry>> {
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Create the status channel a spawner threads into [`register`].
///
/// The spawner moves the [`watch::Sender`] into its detached task and `send`s a
/// terminal [`SubagentStatus`] on completion. Dropping the sender (e.g. a
/// panicked/aborted task) closes the channel, which `wait_subagent` surfaces as
/// a failure rather than hanging.
pub(crate) fn status_channel() -> (
    watch::Sender<SubagentStatus>,
    watch::Receiver<SubagentStatus>,
) {
    watch::channel(SubagentStatus::Running)
}

/// Register a running async sub-agent so it can be steered and waited on.
///
/// Call this *after* `tokio::spawn` so the [`AbortHandle`] is available; the
/// task owns the matching [`watch::Sender`] from [`status_channel`]. Once the
/// table passes [`REGISTRY_SOFT_CAP`], registration sweeps already-terminal
/// entries so it stays bounded even if a parent never calls `wait_subagent`.
pub(crate) fn register(
    task_id: String,
    agent_id: String,
    parent_session: String,
    session_parent_prefix: Option<String>,
    subagent_session_id: Option<String>,
    workspace_dir: PathBuf,
    parent_thread_id: Option<String>,
    run_queue: Arc<RunQueue>,
    abort: AbortHandle,
    status: watch::Receiver<SubagentStatus>,
) {
    // Typed lifecycle ledger: record the spawn and mirror the child's terminal
    // status into the store via a lightweight watcher (issue #4249). Done before
    // the entry is moved into the map so the metadata is still in scope.
    record_spawned(
        &task_id,
        &agent_id,
        &parent_session,
        session_parent_prefix.as_deref(),
        subagent_session_id.as_deref(),
        &workspace_dir,
        parent_thread_id.as_deref(),
    );
    spawn_status_watcher(task_id.clone(), workspace_dir.clone(), status.clone());

    let entry = RunningSubagentEntry {
        agent_id,
        parent_session,
        subagent_session_id,
        workspace_dir,
        parent_thread_id,
        run_queue,
        abort,
        // Fresh cooperative-cancel token registered alongside the abort handle.
        // Threading it into the child run's `RunContext` (so cooperative cancel
        // reaches the executor loop) is part of the gated executor shrink; today
        // it establishes the cancel channel + terminal store write on the cancel
        // paths without disturbing abort-handle hard-kill.
        cancel: CancellationToken::new(),
        status,
    };
    let mut map = registry().lock().expect("running_subagents mutex poisoned");
    if map.len() >= REGISTRY_SOFT_CAP {
        // Only under genuine pressure: sweep collected/terminal entries so the
        // table can't grow without bound when a parent never waits (the Codex
        // spawn-slot leak). Live (Running) entries are always retained.
        map.retain(|task_id, e| {
            let keep = !e.status.borrow().is_terminal();
            if !keep {
                deregister_steering(task_id);
            }
            keep
        });
    }
    map.insert(task_id.clone(), entry);
    log::debug!(
        "[running_subagents] registered task_id={} live_entries={}",
        task_id,
        map.len()
    );
}

/// Watch a child's status channel and mirror the first terminal status into the
/// typed lifecycle store. A dropped sender (aborted/panicked task) without a
/// terminal status is recorded as a failure, matching [`wait`].
fn spawn_status_watcher(
    task_id: String,
    workspace_dir: PathBuf,
    mut status: watch::Receiver<SubagentStatus>,
) {
    tokio::spawn(async move {
        loop {
            let snapshot = status.borrow_and_update().clone();
            if snapshot.is_terminal() {
                record_status(&workspace_dir, &task_id, &snapshot);
                break;
            }
            if status.changed().await.is_err() {
                record_status(
                    &workspace_dir,
                    &task_id,
                    &SubagentStatus::Failed {
                        error: "sub-agent task ended without reporting a result".to_string(),
                    },
                );
                break;
            }
        }
    });
}

/// Compact, read-only view of one registered sub-agent, for ambient injection
/// into a parent's turn context (see [`active_subagents_context_block`]).
#[derive(Debug, Clone)]
pub(crate) struct SubagentSnapshot {
    /// Worker *type* (e.g. `researcher`). Not unique — two parallel researchers
    /// share this; disambiguate on `subagent_session_id` / `task_id`.
    pub(crate) agent_id: String,
    /// Durable, stable per-worker reference the prompt steers/waits/closes by.
    pub(crate) subagent_session_id: Option<String>,
    /// Transient registry key.
    pub(crate) task_id: String,
    /// Stable status label: `running` / `awaiting_user` / `completed` / `failed`.
    pub(crate) status: &'static str,
}

/// Snapshot the sub-agents registered under `parent_session`, with each status
/// read live from its watch channel. Read-only: it takes the registry lock only
/// long enough to clone out the small summaries, never blocks on a child, and
/// never mutates the table. Ordered by `agent_id` then `task_id` so the rendered
/// roster is stable across turns (the underlying map is unordered).
pub(crate) fn snapshot_for_parent(parent_session: &str) -> Vec<SubagentSnapshot> {
    let map = registry().lock().expect("running_subagents mutex poisoned");
    let mut out: Vec<SubagentSnapshot> = map
        .iter()
        .filter(|(_, entry)| entry.parent_session == parent_session)
        .map(|(task_id, entry)| {
            let status = match &*entry.status.borrow() {
                SubagentStatus::Running => "running",
                SubagentStatus::Completed { .. } => "completed",
                SubagentStatus::AwaitingUser { .. } => "awaiting_user",
                SubagentStatus::Failed { .. } => "failed",
            };
            SubagentSnapshot {
                agent_id: entry.agent_id.clone(),
                subagent_session_id: entry.subagent_session_id.clone(),
                task_id: task_id.clone(),
                status,
            }
        })
        .collect();
    out.sort_by(|a, b| {
        a.agent_id
            .cmp(&b.agent_id)
            .then_with(|| a.task_id.cmp(&b.task_id))
    });
    out
}

/// Build the ambient `[active_subagents]` block prepended to a parent's turn
/// context when it has async/parallel workers registered. Returns `None` when the
/// parent owns no registered sub-agents, so the block only appears when it is
/// actionable — turns for agents that never spawn are untouched. Mirrors the
/// thread-goal `[active_goal]` block: it rides the per-turn context (not the
/// cached system-prompt prefix), so it reflects live status every turn.
pub(crate) fn active_subagents_context_block(parent_session: &str) -> Option<String> {
    let workers = snapshot_for_parent(parent_session);
    if workers.is_empty() {
        return None;
    }
    let mut block = format!(
        "[active_subagents]\n\
         You have {} background sub-agent(s) in flight (spawned earlier this session). \
         This is your live roster — trust it over memory. Track each by subagent_session_id; \
         use wait_subagent to collect a `completed` one, steer_subagent to redirect a `running` \
         one, continue_subagent to answer an `awaiting_user` one, close_subagent when done, and \
         list_subagents to re-enumerate. Never fabricate a result for a worker still running or \
         one that has failed.\n",
        workers.len()
    );
    for w in &workers {
        let session = w.subagent_session_id.as_deref().unwrap_or("(none)");
        block.push_str(&format!(
            "- {} · session={} · task={} · status={}\n",
            w.agent_id, session, w.task_id, w.status
        ));
    }
    block.push_str("[/active_subagents]\n\n");
    Some(block)
}

/// Resolve a durable `subagent_session_id` to the currently-running transient
/// `task_id`, enforcing parent-session ownership.
pub(crate) fn task_id_for_session(
    subagent_session_id: &str,
    parent_session: &str,
) -> Result<String, WaitError> {
    let map = registry().lock().expect("running_subagents mutex poisoned");
    let mut saw_unowned = false;
    let mut owned_terminal: Option<String> = None;
    for (task_id, entry) in map
        .iter()
        .filter(|(_, entry)| entry.subagent_session_id.as_deref() == Some(subagent_session_id))
    {
        if entry.parent_session != parent_session {
            saw_unowned = true;
            continue;
        }
        if !entry.status.borrow().is_terminal() {
            return Ok(task_id.clone());
        }
        owned_terminal.get_or_insert_with(|| task_id.clone());
    }
    if let Some(task_id) = owned_terminal {
        return Ok(task_id);
    }
    if saw_unowned {
        return Err(WaitError::NotOwned);
    }
    Err(WaitError::Unknown)
}

pub(crate) fn task_id_for_session_in_workspace(
    subagent_session_id: &str,
    parent_session: &str,
    workspace_dir: &Path,
) -> Result<String, WaitError> {
    match task_id_for_session(subagent_session_id, parent_session) {
        Ok(task_id) => return Ok(task_id),
        Err(WaitError::NotOwned) => return Err(WaitError::NotOwned),
        Err(WaitError::Unknown) => {}
    }

    let mut saw_unowned = false;
    let mut matches: Vec<OrchestrationTaskRecord> = list_task_records(workspace_dir)
        .into_iter()
        .filter(|record| record_subagent_session_id(record) == Some(subagent_session_id))
        .collect();
    matches.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

    for record in matches {
        if record_parent_session(&record) != Some(parent_session) {
            saw_unowned = true;
            continue;
        }
        let task_id = record.spec.task_id.as_str().to_string();
        log::debug!(
            "[running_subagents] resolved session from task store subagent_session_id={} task_id={} workspace_dir={}",
            subagent_session_id,
            task_id,
            workspace_dir.display()
        );
        return Ok(task_id);
    }
    if saw_unowned {
        return Err(WaitError::NotOwned);
    }
    Err(WaitError::Unknown)
}

pub(crate) fn resume_ref_for_task(
    task_id: &str,
    parent_session: &str,
) -> Result<SubagentResumeRef, WaitError> {
    let map = registry().lock().expect("running_subagents mutex poisoned");
    let entry = map.get(task_id).ok_or(WaitError::Unknown)?;
    if entry.parent_session != parent_session {
        return Err(WaitError::NotOwned);
    }
    Ok(SubagentResumeRef {
        task_id: task_id.to_string(),
        agent_id: entry.agent_id.clone(),
        subagent_session_id: entry.subagent_session_id.clone(),
    })
}

pub(crate) fn resume_ref_for_task_in_workspace(
    task_id: &str,
    parent_session: &str,
    workspace_dir: &Path,
) -> Result<SubagentResumeRef, WaitError> {
    match resume_ref_for_task(task_id, parent_session) {
        Ok(reference) => return Ok(reference),
        Err(WaitError::NotOwned) => return Err(WaitError::NotOwned),
        Err(WaitError::Unknown) => {}
    }

    let record = task_record_for_task_in_workspace(workspace_dir, task_id, parent_session)?;
    log::debug!(
        "[running_subagents] resolved resume ref from task store task_id={} workspace_dir={}",
        task_id,
        workspace_dir.display()
    );
    Ok(SubagentResumeRef {
        task_id: task_id.to_string(),
        agent_id: record_agent_id(&record),
        subagent_session_id: record_subagent_session_id(&record).map(ToOwned::to_owned),
    })
}

/// Why a steer could not be delivered.
#[derive(Debug, PartialEq, Eq)]
pub enum SteerError {
    /// No such sub-agent — never existed, or already finished and pruned.
    Unknown,
    /// The caller's `parent_session` does not own this sub-agent.
    NotOwned,
    /// The sub-agent already reached a terminal status.
    AlreadyDone,
}

fn steering_command_for_mode(mode: QueueMode, text: String) -> Option<SteeringCommand> {
    match mode {
        QueueMode::Steer => Some(SteeringCommand::InjectMessage(TaMessage::user(format!(
            "[User steering message]: {text}"
        )))),
        QueueMode::Collect => Some(SteeringCommand::InjectMessage(TaMessage::user(format!(
            "[Additional context from user]: {text}"
        )))),
        QueueMode::Interrupt | QueueMode::Followup | QueueMode::Parallel => None,
    }
}

fn send_registered_steering(task_id: &str, text: String, mode: QueueMode) -> bool {
    let Some(command) = steering_command_for_mode(mode, text) else {
        return false;
    };
    let task_id = TaskId::new(task_id);
    let Some(handle) = shared_steering_registry().get(&task_id) else {
        return false;
    };
    handle.send(command);
    true
}

/// Crate-native steering directives beyond the `InjectMessage`/collect lanes.
///
/// These map 1:1 onto the tinyagents [`SteeringCommand`] control variants that
/// the crate exposes (`Redirect`, `Pause`, `Resume`, `Cancel`). They are
/// delivered **only** through a registered [`SteeringHandle`] and therefore land
/// only at a safe loop boundary (the crate drains before each model call) —
/// never mid-stream, and never through the `RunQueue` fallback (which has no
/// equivalent lane). Approval/security is never bypassed: `Redirect` lowers to a
/// system instruction the normal approval-gated loop still governs, and
/// `Pause`/`Resume`/`Cancel` are pure control-flow.
///
/// The crate's `SetMetadata` command is intentionally *not* mapped here: no
/// OpenHuman control surface owns run-metadata mutation yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SteeringDirective {
    /// Redirect the run toward a new instruction (`SteeringCommand::Redirect`).
    Redirect(String),
    /// Cooperatively pause at the next checkpoint (`SteeringCommand::Pause`).
    Pause,
    /// Clear a pending pause (`SteeringCommand::Resume`).
    Resume,
    /// Cooperatively terminate at the next checkpoint (`SteeringCommand::Cancel`) —
    /// a graceful, safe-boundary alternative to the hard `AbortHandle` cancel.
    Cancel,
}

impl SteeringDirective {
    fn kind(&self) -> SteeringCommandKind {
        match self {
            SteeringDirective::Redirect(_) => SteeringCommandKind::Redirect,
            SteeringDirective::Pause => SteeringCommandKind::Pause,
            SteeringDirective::Resume => SteeringCommandKind::Resume,
            SteeringDirective::Cancel => SteeringCommandKind::Cancel,
        }
    }

    fn into_command(self) -> SteeringCommand {
        match self {
            SteeringDirective::Redirect(instruction) => SteeringCommand::Redirect { instruction },
            SteeringDirective::Pause => SteeringCommand::Pause,
            SteeringDirective::Resume => SteeringCommand::Resume,
            SteeringDirective::Cancel => SteeringCommand::Cancel,
        }
    }
}

/// Why a crate-native steering directive could not be delivered.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SteerDirectiveError {
    /// No such sub-agent — never existed, or already finished and pruned.
    Unknown,
    /// The caller's `parent_session` does not own this sub-agent.
    NotOwned,
    /// The sub-agent already reached a terminal status.
    AlreadyDone,
    /// The sub-agent has no live crate-native `SteeringHandle` registered
    /// (e.g. a legacy `RunQueue`-only run), so control-flow steering that has no
    /// `RunQueue` lane cannot be delivered.
    NoRegisteredHandle,
    /// The run's [`SteeringPolicy`] does not permit this directive's command
    /// kind. Enqueuing it anyway would abort the run with
    /// `TinyAgentsError::Steering`, so we refuse up front.
    PolicyRejected,
}

/// Deliver a crate-native control-flow [`SteeringDirective`] to a running
/// sub-agent through its registered TinyAgents [`SteeringHandle`].
///
/// Unlike [`steer`], this has **no** `RunQueue` fallback: the crate control
/// variants (`Redirect`/`Pause`/`Resume`/`Cancel`) have no OpenHuman queue lane,
/// so a run must have a live registered handle to receive them. The directive's
/// command kind is checked against the run's own `SteeringPolicy` *before*
/// enqueue — a disallowed command would otherwise abort the run — so this can
/// never smuggle a control kind past a policy that a tighter run class installed.
pub(crate) fn steer_directive(
    task_id: &str,
    parent_session: &str,
    directive: SteeringDirective,
) -> Result<(), SteerDirectiveError> {
    {
        let map = registry().lock().expect("running_subagents mutex poisoned");
        let entry = map.get(task_id).ok_or(SteerDirectiveError::Unknown)?;
        if entry.parent_session != parent_session {
            return Err(SteerDirectiveError::NotOwned);
        }
        if entry.status.borrow().is_terminal() {
            return Err(SteerDirectiveError::AlreadyDone);
        }
    }

    let handle = shared_steering_registry()
        .get(&TaskId::new(task_id))
        .ok_or(SteerDirectiveError::NoRegisteredHandle)?;
    let kind = directive.kind();
    if !handle.policy().is_allowed(kind) {
        log::warn!(
            "[running_subagents] directive rejected by run policy task_id={} kind={}",
            task_id,
            kind.as_str()
        );
        return Err(SteerDirectiveError::PolicyRejected);
    }
    handle.send(directive.into_command());
    log::info!(
        "[running_subagents] steered task_id={} directive={} via=tinyagents_registry",
        task_id,
        kind.as_str()
    );
    Ok(())
}

fn deregister_steering(task_id: &str) {
    let task_id = TaskId::new(task_id);
    if shared_steering_registry().deregister(&task_id).is_some() {
        log::debug!(
            "[running_subagents] deregistered steering handle task_id={}",
            task_id.as_str()
        );
    }
}

/// Inject a message into a running sub-agent. Prefer the crate-native
/// TinyAgents steering registry when the child run has registered its live
/// handle, and fall back to the OpenHuman `RunQueue` compatibility path.
pub async fn steer(
    task_id: &str,
    parent_session: &str,
    text: String,
    mode: QueueMode,
) -> Result<(), SteerError> {
    let run_queue = {
        let map = registry().lock().expect("running_subagents mutex poisoned");
        let entry = map.get(task_id).ok_or(SteerError::Unknown)?;
        if entry.parent_session != parent_session {
            return Err(SteerError::NotOwned);
        }
        if entry.status.borrow().is_terminal() {
            return Err(SteerError::AlreadyDone);
        }
        entry.run_queue.clone()
    };

    if send_registered_steering(task_id, text.clone(), mode) {
        log::info!(
            "[running_subagents] steered task_id={} mode={} via=tinyagents_registry",
            task_id,
            mode
        );
        return Ok(());
    }

    run_queue
        .push(QueuedMessage {
            text,
            mode,
            client_id: "steer_subagent".to_string(),
            thread_id: task_id.to_string(),
            queued_at_ms: now_ms(),
            model_override: None,
            temperature: None,
            profile_id: None,
            locale: None,
        })
        .await;
    log::info!(
        "[running_subagents] steered task_id={} mode={}",
        task_id,
        mode
    );
    Ok(())
}

/// Trusted-control variant used by JSON-RPC sub-agent controls.
///
/// This intentionally does not require the caller to provide `parent_session`:
/// the RPC layer is already bearer-protected and mirrors the existing
/// `subagent_cancel` control surface, which can abort a task by id. The function
/// still refuses unknown or terminal tasks and never logs the steered text.
pub(crate) async fn steer_control(
    task_id: &str,
    text: String,
    mode: QueueMode,
) -> Result<(), SteerError> {
    let run_queue = {
        let map = registry().lock().expect("running_subagents mutex poisoned");
        let entry = map.get(task_id).ok_or(SteerError::Unknown)?;
        if entry.status.borrow().is_terminal() {
            return Err(SteerError::AlreadyDone);
        }
        entry.run_queue.clone()
    };

    if send_registered_steering(task_id, text.clone(), mode) {
        log::info!(
            "[running_subagents] control_steered task_id={} mode={} via=tinyagents_registry",
            task_id,
            mode
        );
        return Ok(());
    }

    run_queue
        .push(QueuedMessage {
            text,
            mode,
            client_id: "subagent_control_rpc".to_string(),
            thread_id: task_id.to_string(),
            queued_at_ms: now_ms(),
            model_override: None,
            temperature: None,
            profile_id: None,
            locale: None,
        })
        .await;
    log::info!(
        "[running_subagents] control_steered task_id={} mode={}",
        task_id,
        mode
    );
    Ok(())
}

/// Why a wait could not be set up.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum WaitError {
    Unknown,
    NotOwned,
}

/// Result of waiting on a sub-agent.
#[derive(Debug)]
pub(crate) enum WaitOutcome {
    /// The sub-agent reached a terminal status (entry pruned).
    Terminal(SubagentStatus),
    /// The timeout elapsed first; the entry is left intact so the parent can
    /// wait again. Carries the latest (non-terminal) status snapshot.
    TimedOut(SubagentStatus),
}

/// Block until `task_id` reaches a terminal status or `timeout` elapses.
pub(crate) async fn wait(
    task_id: &str,
    parent_session: &str,
    timeout: Duration,
) -> Result<WaitOutcome, WaitError> {
    let mut rx = {
        let map = registry().lock().expect("running_subagents mutex poisoned");
        let entry = map.get(task_id).ok_or(WaitError::Unknown)?;
        if entry.parent_session != parent_session {
            return Err(WaitError::NotOwned);
        }
        entry.status.clone()
    };

    // Fast path: already terminal.
    let current = rx.borrow_and_update().clone();
    if current.is_terminal() {
        prune(task_id);
        return Ok(WaitOutcome::Terminal(current));
    }

    let waited = async {
        loop {
            if rx.changed().await.is_err() {
                // Sender dropped without a terminal status (task aborted/panicked).
                return SubagentStatus::Failed {
                    error: "sub-agent task ended without reporting a result".to_string(),
                };
            }
            let status = rx.borrow().clone();
            if status.is_terminal() {
                return status;
            }
        }
    };

    match tokio::time::timeout(timeout, waited).await {
        Ok(status) => {
            prune(task_id);
            Ok(WaitOutcome::Terminal(status))
        }
        Err(_) => Ok(WaitOutcome::TimedOut(rx.borrow().clone())),
    }
}

pub(crate) async fn wait_in_workspace(
    task_id: &str,
    parent_session: &str,
    workspace_dir: &Path,
    timeout: Duration,
) -> Result<WaitOutcome, WaitError> {
    match wait(task_id, parent_session, timeout).await {
        Ok(outcome) => return Ok(outcome),
        Err(WaitError::NotOwned) => return Err(WaitError::NotOwned),
        Err(WaitError::Unknown) => {}
    }

    let record = task_record_for_task_in_workspace(workspace_dir, task_id, parent_session)?;
    log::debug!(
        "[running_subagents] resolved wait from task store task_id={} status={} workspace_dir={}",
        task_id,
        task_status_label(record.status),
        workspace_dir.display()
    );
    Ok(record_to_status(record))
}

/// Metadata captured when a sub-agent is cancelled, so the caller can surface
/// the cancellation back in the parent chat (record a "cancelled" completion
/// for idle-gated delivery).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CancelledSubagent {
    pub(crate) agent_id: String,
    pub(crate) parent_session: String,
    pub(crate) subagent_session_id: Option<String>,
    pub(crate) workspace_dir: PathBuf,
    pub(crate) parent_thread_id: Option<String>,
}

/// Abort and drop the sub-agent with `task_id`, returning its metadata so the
/// caller can deliver a "cancelled" notice into the parent chat. Returns `None`
/// if no such sub-agent is registered (already finished, or unknown id).
///
/// Unlike the parent-session-owned steering and close paths, this is keyed by
/// `task_id` alone with no ownership check — it backs the user-facing "Cancel"
/// affordance, and the desktop user owns every sub-agent in their own core.
pub(crate) fn cancel_by_task(task_id: &str) -> Option<CancelledSubagent> {
    let mut map = registry().lock().expect("running_subagents mutex poisoned");
    let entry = map.remove(task_id)?;
    deregister_steering(task_id);
    // Cooperative cancel first (safe-boundary unwind for opted-in runs), then the
    // hard abort as the executor-detail stop, then the terminal store write.
    entry.cancel.cancel();
    entry.abort.abort();
    record_cancelled(&entry.workspace_dir, task_id);
    log::debug!(
        "[running_subagents] cancel_by_task task_id={} agent_id={} parent_thread_id={:?} live_entries={}",
        task_id,
        entry.agent_id,
        entry.parent_thread_id,
        map.len()
    );
    Some(CancelledSubagent {
        agent_id: entry.agent_id,
        parent_session: entry.parent_session,
        subagent_session_id: entry.subagent_session_id,
        workspace_dir: entry.workspace_dir,
        parent_thread_id: entry.parent_thread_id,
    })
}

pub(crate) fn cancel_by_session(
    subagent_session_id: &str,
    parent_session: &str,
) -> Option<CancelledSubagent> {
    let task_id = task_id_for_session(subagent_session_id, parent_session).ok()?;
    cancel_by_task(&task_id)
}

pub(crate) fn cancel_by_session_in_workspace(
    subagent_session_id: &str,
    parent_session: &str,
    workspace_dir: &Path,
) -> Option<CancelledSubagent> {
    let task_id =
        task_id_for_session_in_workspace(subagent_session_id, parent_session, workspace_dir)
            .ok()?;
    cancel_by_task(&task_id)
}

/// Abort and drop every running sub-agent whose parent chat thread is
/// `thread_id`. Called when that thread is deleted so detached children don't
/// keep running (and later try to deliver) against a thread that no longer
/// exists. Returns the number of sub-agents cancelled.
pub(crate) fn cancel_for_thread(thread_id: &str) -> usize {
    let mut map = registry().lock().expect("running_subagents mutex poisoned");
    let to_cancel: Vec<String> = map
        .iter()
        .filter(|(_, e)| e.parent_thread_id.as_deref() == Some(thread_id))
        .map(|(id, _)| id.clone())
        .collect();
    for id in &to_cancel {
        if let Some(entry) = map.remove(id) {
            deregister_steering(id);
            // Cooperative cancel before the hard abort (issue #4249 / 07.2 step 2),
            // mirroring `cancel_by_task`, then the terminal store write.
            entry.cancel.cancel();
            entry.abort.abort();
            record_cancelled(&entry.workspace_dir, id);
        }
    }
    let count = to_cancel.len();
    log::debug!(
        "[running_subagents] cancel_for_thread thread_id={} cancelled={} live_entries={}",
        thread_id,
        count,
        map.len()
    );
    count
}

/// Abort and drop **every** registered sub-agent. Called on a full thread purge
/// where no parent thread survives. Returns the **distinct parent thread ids**
/// that had sub-agents, so the purge path can tombstone them in
/// [`super::background_completions`] and drop any straggler completion that wins
/// the cooperative-abort race. Headless sub-agents (no parent thread) are still
/// aborted but contribute no id.
pub(crate) fn cancel_all() -> Vec<String> {
    let mut map = registry().lock().expect("running_subagents mutex poisoned");
    let count = map.len();
    let mut thread_ids: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for (task_id, entry) in map.drain() {
        deregister_steering(&task_id);
        // Cooperative cancel before the hard abort (issue #4249 / 07.2 step 2),
        // mirroring `cancel_by_task`, then the terminal store write.
        entry.cancel.cancel();
        entry.abort.abort();
        record_cancelled(&entry.workspace_dir, &task_id);
        if let Some(thread_id) = entry.parent_thread_id {
            if seen.insert(thread_id.clone()) {
                thread_ids.push(thread_id);
            }
        }
    }
    log::debug!(
        "[running_subagents] cancel_all cancelled={} distinct_threads={}",
        count,
        thread_ids.len()
    );
    thread_ids
}

fn prune(task_id: &str) {
    deregister_steering(task_id);
    registry()
        .lock()
        .expect("running_subagents mutex poisoned")
        .remove(task_id);
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::tinyagents::orchestration::{
        openhuman_steering_handle, OrchestrationTaskStatus, SteeringHandle, SteeringPolicy,
        SteeringRunClass,
    };
    use std::sync::MutexGuard;

    /// Serializes every test that touches the global [`REGISTRY`]. We reuse the
    /// crate-wide `TEST_ENV_LOCK` (rather than a module-local mutex) because the
    /// destructive `cancel_all` path is also reachable from the `threads::ops`
    /// tests — those hold the same lock, so this prevents a purge there from
    /// wiping entries a test here is mid-way through.
    fn test_guard() -> MutexGuard<'static, ()> {
        // Recover from a poisoned guard so one panicking test doesn't cascade.
        crate::openhuman::config::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    fn dummy_abort() -> AbortHandle {
        tokio::spawn(async {}).abort_handle()
    }

    /// Per-process-run unique workspace for the detached task store.
    ///
    /// The task store is now a durable JSONL file under the given workspace
    /// (issue #4249). Pointing tests at the shared `std::env::temp_dir()` leaked
    /// records **across** test-process runs: a `task-ledger-1` left `Completed`
    /// by a prior run would make `record_spawned`'s insert/`mark_running` no-op
    /// (id already terminal), so this run would observe the stale terminal status
    /// instead of `Running`. A fresh temp dir per process run keeps the store
    /// hermetic; task ids are unique across tests so a single shared dir is safe
    /// within a run. The `TempDir` lives for the whole process (cleaned at exit).
    fn test_workspace() -> PathBuf {
        static WORKSPACE: std::sync::LazyLock<tempfile::TempDir> = std::sync::LazyLock::new(|| {
            tempfile::tempdir().expect("create hermetic test task-store workspace")
        });
        WORKSPACE.path().to_path_buf()
    }

    /// Register a sub-agent for tests, returning the status sender so the test
    /// can drive completion. Keeping the sender alive keeps the channel open.
    fn register_test(
        task_id: &str,
        parent_session: &str,
        rq: Arc<RunQueue>,
    ) -> watch::Sender<SubagentStatus> {
        register_test_with_thread(task_id, parent_session, None, rq)
    }

    /// Like [`register_test`] but lets a test set the parent thread id so it can
    /// exercise [`cancel_for_thread`].
    fn register_test_with_thread(
        task_id: &str,
        parent_session: &str,
        parent_thread_id: Option<&str>,
        rq: Arc<RunQueue>,
    ) -> watch::Sender<SubagentStatus> {
        let (tx, rx) = status_channel();
        register(
            task_id.into(),
            "researcher".into(),
            parent_session.into(),
            None,
            None,
            test_workspace(),
            parent_thread_id.map(Into::into),
            rq,
            dummy_abort(),
            rx,
        );
        tx
    }

    #[tokio::test]
    async fn task_store_records_spawn_complete_and_cancel() {
        let _guard = test_guard();
        // Spawn → the ledger sees a running SubAgent task scoped to the parent.
        let tx = register_test("task-ledger-1", "ledger-parent", RunQueue::new());
        let running = task_records(Some("ledger-parent"));
        assert!(
            running
                .iter()
                .any(|r| r.spec.task_id.as_str() == "task-ledger-1"
                    && r.spec.parent_run_id.as_ref().map(|id| id.as_str())
                        == Some("ledger-parent")
                    && r.spec.root_run_id.as_ref().map(|id| id.as_str()) == Some("ledger-parent")
                    && r.spec.timeout_ms == Some(DETACHED_LEDGER_TIMEOUT_MS)
                    && r.status == OrchestrationTaskStatus::Running),
            "spawned sub-agent is recorded Running: {running:?}"
        );

        // Publish a terminal status → the watcher mirrors Completed into the store.
        tx.send(SubagentStatus::Completed {
            output: "done".into(),
            iterations: 2,
        })
        .unwrap();
        // Let the watcher task observe the change.
        for _ in 0..50 {
            tokio::task::yield_now().await;
            if task_records(None)
                .iter()
                .any(|r| r.spec.task_id.as_str() == "task-ledger-1" && r.is_terminal())
            {
                break;
            }
        }
        let after = task_records(None);
        let rec = after
            .iter()
            .find(|r| r.spec.task_id.as_str() == "task-ledger-1")
            .expect("ledger record present");
        assert_eq!(rec.status, OrchestrationTaskStatus::Completed);

        // A second sub-agent that gets cancelled is recorded Cancelled.
        let _tx2 = register_test("task-ledger-2", "ledger-parent", RunQueue::new());
        assert!(cancel_by_task("task-ledger-2").is_some());
        let cancelled = task_records(None)
            .into_iter()
            .find(|r| r.spec.task_id.as_str() == "task-ledger-2")
            .expect("cancelled record present");
        assert_eq!(cancelled.status, OrchestrationTaskStatus::Cancelled);

        prune("task-ledger-1");
    }

    #[tokio::test]
    async fn task_id_for_session_enforces_parent_ownership() {
        let _guard = test_guard();
        let rq = RunQueue::new();
        let (tx, rx) = status_channel();
        register(
            "task-session".into(),
            "researcher".into(),
            "session-owner".into(),
            None,
            Some("subsess-1".into()),
            test_workspace(),
            Some("thread-1".into()),
            rq,
            dummy_abort(),
            rx,
        );

        assert_eq!(
            task_id_for_session("subsess-1", "session-owner").unwrap(),
            "task-session"
        );
        assert!(matches!(
            task_id_for_session("subsess-1", "session-other"),
            Err(WaitError::NotOwned)
        ));
        let _ = tx.send(SubagentStatus::Completed {
            output: "done".into(),
            iterations: 1,
        });
        prune("task-session");
    }

    #[tokio::test]
    async fn snapshot_and_block_scope_to_parent_and_reflect_live_status() {
        let _guard = test_guard();
        let (tx_a, rx_a) = status_channel();
        register(
            "task-fleet-a".into(),
            "researcher".into(),
            "fleet-parent".into(),
            None,
            Some("subsess-a".into()),
            test_workspace(),
            None,
            RunQueue::new(),
            dummy_abort(),
            rx_a,
        );
        let (tx_b, rx_b) = status_channel();
        register(
            "task-fleet-b".into(),
            "code_executor".into(),
            "fleet-parent".into(),
            None,
            Some("subsess-b".into()),
            test_workspace(),
            None,
            RunQueue::new(),
            dummy_abort(),
            rx_b,
        );
        // A worker owned by a different parent must not leak into the snapshot.
        let (tx_other, rx_other) = status_channel();
        register(
            "task-fleet-other".into(),
            "researcher".into(),
            "other-parent".into(),
            None,
            Some("subsess-other".into()),
            test_workspace(),
            None,
            RunQueue::new(),
            dummy_abort(),
            rx_other,
        );

        // `b` pauses awaiting the user; `a` stays running.
        tx_b.send(SubagentStatus::AwaitingUser {
            question: "which repo?".into(),
        })
        .unwrap();

        let snap = snapshot_for_parent("fleet-parent");
        assert_eq!(snap.len(), 2, "only this parent's workers: {snap:?}");
        // Ordered by agent_id then task_id → code_executor before researcher.
        assert_eq!(snap[0].agent_id, "code_executor");
        assert_eq!(snap[0].status, "awaiting_user");
        assert_eq!(snap[1].agent_id, "researcher");
        assert_eq!(snap[1].status, "running");

        let block = active_subagents_context_block("fleet-parent").expect("block present");
        assert!(block.contains("[active_subagents]"));
        assert!(block.contains("You have 2 background sub-agent(s)"));
        assert!(block.contains("session=subsess-a"));
        assert!(block.contains("session=subsess-b · task=task-fleet-b · status=awaiting_user"));
        assert!(block.ends_with("[/active_subagents]\n\n"));

        // A parent with no registered workers gets no block (no perturbation).
        assert!(active_subagents_context_block("nobody-here").is_none());

        let _ = tx_a.send(SubagentStatus::Completed {
            output: "x".into(),
            iterations: 1,
        });
        let _ = tx_other.send(SubagentStatus::Completed {
            output: "x".into(),
            iterations: 1,
        });
        prune("task-fleet-a");
        prune("task-fleet-b");
        prune("task-fleet-other");
    }

    #[tokio::test]
    async fn resume_ref_for_task_includes_resume_fields_and_enforces_ownership() {
        let _guard = test_guard();
        let (tx, rx) = status_channel();
        register(
            "task-resume".into(),
            "researcher".into(),
            "session-owner".into(),
            None,
            Some("subsess-resume".into()),
            test_workspace(),
            Some("thread-1".into()),
            RunQueue::new(),
            dummy_abort(),
            rx,
        );

        let reference =
            resume_ref_for_task("task-resume", "session-owner").expect("resume reference");
        assert_eq!(reference.task_id, "task-resume");
        assert_eq!(reference.agent_id, "researcher");
        assert_eq!(
            reference.subagent_session_id.as_deref(),
            Some("subsess-resume")
        );
        assert!(matches!(
            resume_ref_for_task("task-resume", "session-other"),
            Err(WaitError::NotOwned)
        ));

        let _ = tx.send(SubagentStatus::Completed {
            output: "done".into(),
            iterations: 1,
        });
        prune("task-resume");
    }

    #[tokio::test]
    async fn task_id_for_session_prefers_live_task_over_terminal_task() {
        let _guard = test_guard();
        let (old_tx, old_rx) = status_channel();
        register(
            "task-old".into(),
            "researcher".into(),
            "session-owner".into(),
            None,
            Some("subsess-live".into()),
            test_workspace(),
            Some("thread-1".into()),
            RunQueue::new(),
            dummy_abort(),
            old_rx,
        );
        let _ = old_tx.send(SubagentStatus::Completed {
            output: "old".into(),
            iterations: 1,
        });
        let (_new_tx, new_rx) = status_channel();
        register(
            "task-new".into(),
            "researcher".into(),
            "session-owner".into(),
            None,
            Some("subsess-live".into()),
            test_workspace(),
            Some("thread-1".into()),
            RunQueue::new(),
            dummy_abort(),
            new_rx,
        );

        assert_eq!(
            task_id_for_session("subsess-live", "session-owner").unwrap(),
            "task-new"
        );
        prune("task-old");
        prune("task-new");
    }

    #[tokio::test]
    async fn steer_pushes_into_the_subagent_queue() {
        let _guard = test_guard();
        let rq = RunQueue::new();
        let tx = register_test("task-steer", "session-A", rq.clone());

        steer(
            "task-steer",
            "session-A",
            "refocus on memory safety".into(),
            QueueMode::Steer,
        )
        .await
        .expect("steer should succeed");

        let status = rq.status().await;
        assert_eq!(status.steers, 1, "steer should land in the steer lane");

        // collect mode goes to the collect lane
        steer(
            "task-steer",
            "session-A",
            "extra context".into(),
            QueueMode::Collect,
        )
        .await
        .unwrap();
        assert_eq!(rq.status().await.collects, 1);

        let _ = tx.send(SubagentStatus::Completed {
            output: "done".into(),
            iterations: 1,
        });
        prune("task-steer");
    }

    #[tokio::test]
    async fn steer_prefers_registered_tinyagents_handle() {
        let _guard = test_guard();
        let rq = RunQueue::new();
        let tx = register_test("task-registered-steer", "session-A", rq.clone());
        let handle = SteeringHandle::allow_all();
        let task_id = TaskId::new("task-registered-steer");
        shared_steering_registry().register(task_id.clone(), handle.clone());

        steer(
            "task-registered-steer",
            "session-A",
            "refocus".into(),
            QueueMode::Steer,
        )
        .await
        .expect("steer should succeed");

        let status = rq.status().await;
        assert_eq!(status.steers, 0, "registered handle bypasses RunQueue");
        let commands = handle.drain();
        assert_eq!(commands.len(), 1);
        match &commands[0] {
            SteeringCommand::InjectMessage(message) => {
                assert_eq!(message.text(), "[User steering message]: refocus");
            }
            other => panic!("expected injected steering message, got {other:?}"),
        }

        let _ = shared_steering_registry().deregister(&task_id);
        let _ = tx.send(SubagentStatus::Completed {
            output: "done".into(),
            iterations: 1,
        });
        prune("task-registered-steer");
    }

    #[tokio::test]
    async fn steer_directive_delivers_control_flow_via_background_policy() {
        let _guard = test_guard();
        let rq = RunQueue::new();
        let tx = register_test("task-directive", "session-A", rq.clone());
        // A background sub-agent handle accepts Cancel/Redirect/Resume.
        let handle = openhuman_steering_handle(SteeringRunClass::Background);
        let task_id = TaskId::new("task-directive");
        shared_steering_registry().register(task_id.clone(), handle.clone());

        steer_directive(
            "task-directive",
            "session-A",
            SteeringDirective::Redirect("focus on the failing test".into()),
        )
        .expect("redirect should be accepted");
        steer_directive("task-directive", "session-A", SteeringDirective::Cancel)
            .expect("cancel should be accepted");

        // RunQueue is untouched — directives never fall back to it.
        assert_eq!(rq.status().await.steers, 0);
        let commands = handle.drain();
        assert_eq!(commands.len(), 2);
        assert!(matches!(
            &commands[0],
            SteeringCommand::Redirect { instruction } if instruction == "focus on the failing test"
        ));
        assert_eq!(commands[1], SteeringCommand::Cancel);

        let _ = shared_steering_registry().deregister(&task_id);
        let _ = tx.send(SubagentStatus::Completed {
            output: "done".into(),
            iterations: 1,
        });
        prune("task-directive");
    }

    #[tokio::test]
    async fn steer_directive_refuses_kinds_the_policy_rejects() {
        let _guard = test_guard();
        let rq = RunQueue::new();
        let tx = register_test("task-tight", "session-A", rq);
        // An interactive-class handle only allows InjectMessage/Pause, so a
        // Cancel directive must be refused up front rather than enqueued (which
        // would abort the run).
        let handle = SteeringHandle::new(
            SteeringPolicy::new()
                .allow(SteeringCommandKind::InjectMessage)
                .allow(SteeringCommandKind::Pause),
        );
        let task_id = TaskId::new("task-tight");
        shared_steering_registry().register(task_id.clone(), handle.clone());

        assert_eq!(
            steer_directive("task-tight", "session-A", SteeringDirective::Cancel),
            Err(SteerDirectiveError::PolicyRejected)
        );
        // Pause is allowed on the tight policy.
        steer_directive("task-tight", "session-A", SteeringDirective::Pause)
            .expect("pause should be accepted by the tight policy");
        let commands = handle.drain();
        assert_eq!(commands, vec![SteeringCommand::Pause]);

        let _ = shared_steering_registry().deregister(&task_id);
        let _ = tx.send(SubagentStatus::Completed {
            output: "done".into(),
            iterations: 1,
        });
        prune("task-tight");
    }

    #[tokio::test]
    async fn steer_directive_enforces_ownership_and_registration() {
        let _guard = test_guard();
        let rq = RunQueue::new();
        let tx = register_test("task-own", "session-owner", rq);

        // Cross-parent is refused before any handle lookup.
        assert_eq!(
            steer_directive("task-own", "session-intruder", SteeringDirective::Resume),
            Err(SteerDirectiveError::NotOwned)
        );
        // Unknown task id.
        assert_eq!(
            steer_directive("task-missing", "session-owner", SteeringDirective::Resume),
            Err(SteerDirectiveError::Unknown)
        );
        // Owned but no registered crate handle → cannot deliver control-flow.
        assert_eq!(
            steer_directive("task-own", "session-owner", SteeringDirective::Resume),
            Err(SteerDirectiveError::NoRegisteredHandle)
        );

        let _ = tx.send(SubagentStatus::Completed {
            output: "done".into(),
            iterations: 1,
        });
        prune("task-own");
    }

    #[tokio::test]
    async fn steer_rejects_cross_parent_and_unknown() {
        let _guard = test_guard();
        let rq = RunQueue::new();
        let _tx = register_test("task-owned", "session-owner", rq);

        assert_eq!(
            steer(
                "task-owned",
                "session-intruder",
                "x".into(),
                QueueMode::Steer
            )
            .await,
            Err(SteerError::NotOwned)
        );
        assert_eq!(
            steer(
                "task-missing",
                "session-owner",
                "x".into(),
                QueueMode::Steer
            )
            .await,
            Err(SteerError::Unknown)
        );
        prune("task-owned");
    }

    #[tokio::test]
    async fn steer_after_terminal_is_rejected() {
        let _guard = test_guard();
        let rq = RunQueue::new();
        let tx = register_test("task-term", "session-A", rq);
        let _ = tx.send(SubagentStatus::Failed {
            error: "boom".into(),
        });

        assert_eq!(
            steer("task-term", "session-A", "x".into(), QueueMode::Steer).await,
            Err(SteerError::AlreadyDone)
        );
        prune("task-term");
    }

    #[tokio::test]
    async fn wait_returns_completion_once_published() {
        let _guard = test_guard();
        let rq = RunQueue::new();
        let tx = register_test("task-wait", "session-A", rq);

        tokio::spawn(async move {
            let _ = tx.send(SubagentStatus::Completed {
                output: "the answer".into(),
                iterations: 3,
            });
            // keep sender alive until after send
            drop(tx);
        });

        let outcome = wait("task-wait", "session-A", Duration::from_secs(5))
            .await
            .expect("wait should resolve");
        match outcome {
            WaitOutcome::Terminal(SubagentStatus::Completed { output, iterations }) => {
                assert_eq!(output, "the answer");
                assert_eq!(iterations, 3);
            }
            other => panic!("expected completed terminal, got {other:?}"),
        }

        // pruned after a terminal wait
        assert!(matches!(
            wait("task-wait", "session-A", Duration::from_millis(10)).await,
            Err(WaitError::Unknown)
        ));
    }

    #[tokio::test]
    async fn wait_times_out_and_leaves_entry_intact() {
        let _guard = test_guard();
        let rq = RunQueue::new();
        let _tx = register_test("task-slow", "session-A", rq);

        let outcome = wait("task-slow", "session-A", Duration::from_millis(20))
            .await
            .expect("wait should resolve");
        assert!(matches!(
            outcome,
            WaitOutcome::TimedOut(SubagentStatus::Running)
        ));

        // still steerable after a timed-out wait
        assert!(steer(
            "task-slow",
            "session-A",
            "still here".into(),
            QueueMode::Steer
        )
        .await
        .is_ok());
        prune("task-slow");
    }

    #[tokio::test]
    async fn cancel_for_thread_aborts_only_matching_entries() {
        let _guard = test_guard();
        let rq = RunQueue::new();
        let _a = register_test_with_thread("task-tA-1", "session-A", Some("thread-X"), rq.clone());
        let _b = register_test_with_thread("task-tA-2", "session-A", Some("thread-X"), rq.clone());
        // Different thread — must survive.
        let _c = register_test_with_thread("task-tB", "session-A", Some("thread-Y"), rq.clone());
        // Headless (no parent thread) — must survive.
        let _d = register_test_with_thread("task-headless", "session-A", None, rq);

        let cancelled = cancel_for_thread("thread-X");
        assert_eq!(cancelled, 2, "both thread-X entries should be cancelled");

        // The two cancelled entries are gone (steer can't find them).
        assert_eq!(
            steer("task-tA-1", "session-A", "x".into(), QueueMode::Steer).await,
            Err(SteerError::Unknown)
        );
        assert_eq!(
            steer("task-tA-2", "session-A", "x".into(), QueueMode::Steer).await,
            Err(SteerError::Unknown)
        );

        // Non-matching entries stay live and steerable.
        assert!(steer("task-tB", "session-A", "x".into(), QueueMode::Steer)
            .await
            .is_ok());
        assert!(
            steer("task-headless", "session-A", "x".into(), QueueMode::Steer)
                .await
                .is_ok()
        );

        // Idempotent: a second pass cancels nothing.
        assert_eq!(cancel_for_thread("thread-X"), 0);

        prune("task-tB");
        prune("task-headless");
    }

    #[tokio::test]
    async fn cancel_by_task_returns_metadata_and_removes_entry() {
        let _guard = test_guard();
        let rq = RunQueue::new();
        let _tx =
            register_test_with_thread("task-cbt", "session-Z", Some("thread-cbt"), rq.clone());
        let task_id = TaskId::new("task-cbt");
        shared_steering_registry().register(task_id.clone(), SteeringHandle::allow_all());

        let meta = cancel_by_task("task-cbt").expect("known task should cancel");
        assert_eq!(meta.agent_id, "researcher");
        assert_eq!(meta.parent_session, "session-Z");
        assert_eq!(meta.parent_thread_id.as_deref(), Some("thread-cbt"));
        assert!(
            shared_steering_registry().get(&task_id).is_none(),
            "hard cancel should remove the registered steering handle"
        );

        // Entry is gone — steer can no longer find it, and a second cancel is a no-op.
        assert_eq!(
            steer("task-cbt", "session-Z", "x".into(), QueueMode::Steer).await,
            Err(SteerError::Unknown)
        );
        assert!(cancel_by_task("task-cbt").is_none());
        // Unknown ids are simply None.
        assert!(cancel_by_task("never-existed").is_none());
    }

    #[tokio::test]
    async fn cancel_all_clears_everything() {
        let _guard = test_guard();
        let rq = RunQueue::new();
        let _a = register_test_with_thread("task-all-1", "session-A", Some("thread-1"), rq.clone());
        // Headless (no parent thread) — aborted, but contributes no thread id.
        let _b = register_test_with_thread("task-all-2", "session-B", None, rq);

        let cancelled_threads = cancel_all();
        assert!(
            cancelled_threads.contains(&"thread-1".to_string()),
            "cancel_all should report the parent thread of the cancelled sub-agent"
        );
        assert!(
            !cancelled_threads.iter().any(|t| t.is_empty()),
            "headless sub-agents must not contribute an id"
        );

        assert_eq!(
            steer("task-all-1", "session-A", "x".into(), QueueMode::Steer).await,
            Err(SteerError::Unknown)
        );
        assert_eq!(
            steer("task-all-2", "session-B", "x".into(), QueueMode::Steer).await,
            Err(SteerError::Unknown)
        );
        // Registry is empty now.
        assert!(cancel_all().is_empty());
    }
}
