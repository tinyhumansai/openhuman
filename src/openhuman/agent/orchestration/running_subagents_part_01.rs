use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use tokio::sync::watch;
use tokio::task::AbortHandle;

use crate::openhuman::agent::harness::run_queue::{QueueMode, QueuedMessage, RunQueue};
use crate::openhuman::agent::tinyagents::orchestration::{
    open_jsonl_task_store_or_memory, reconcile_orphaned_tasks, shared_steering_registry,
    DetachedTaskRegistry, DetachedTaskRegistryError, DetachedTaskWaitOutcome, InMemoryTaskStore,
    OrchestrationTaskFilter, OrchestrationTaskKind, OrchestrationTaskRecord,
    OrchestrationTaskResult, OrchestrationTaskSpec, OrchestrationTaskStatus, SteeringCommand,
    SteeringCommandKind, TaskStore, TaskStoreRegistry,
};
use tinyagents_harness::ids::TaskId;
use tinyinference::message::Message as TaMessage;
use tinyagents_harness::CancellationToken;

/// Where a workspace's detached-task ledger lives.
///
/// A product path, not a generic one: TinyAgents opens whatever file it is
/// given, and this is where OpenHuman keeps it.
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

/// Process-wide typed lifecycle ledger for detached sub-agents (issue #4249),
/// one durable store per workspace.
///
/// The caching and the durable→memory fallback are TinyAgents'
/// (`TaskStoreRegistry` / `open_jsonl_task_store_or_memory`): opening a second
/// store over the same append log would give two writers with independently
/// replayed state, and a workspace that cannot be written should degrade to an
/// in-memory ledger rather than take orchestration down. What stays here is the
/// path layout above.
static TASK_STORES: OnceLock<TaskStoreRegistry<PathBuf>> = OnceLock::new();

fn task_stores() -> &'static TaskStoreRegistry<PathBuf> {
    TASK_STORES.get_or_init(|| {
        TaskStoreRegistry::new(|workspace_dir: &PathBuf| {
            open_jsonl_task_store_or_memory(&task_store_path(workspace_dir))
        })
    })
}

/// The ledger for `workspace_dir`, opening it on first use.
///
/// A poisoned registry lock is degraded to a throwaway in-memory store rather
/// than propagated: every caller here is a best-effort bookkeeping path, and a
/// panic in an unrelated task must not turn sub-agent spawning into a second
/// panic.
fn task_store_for_workspace(workspace_dir: &Path) -> Arc<dyn TaskStore> {
    let key = workspace_dir.to_path_buf();
    match task_stores().get_or_open(&key) {
        Ok(store) => store,
        Err(err) => {
            log::warn!(
                "[running_subagents] task store registry unavailable for {}; using a detached in-memory ledger: {}",
                workspace_dir.display(),
                err
            );
            Arc::new(InMemoryTaskStore::new())
        }
    }
}

#[cfg(test)]
fn task_store() -> Arc<dyn TaskStore> {
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
/// The reason an orphaned sub-agent record is settled with.
///
/// Built in one place because it is written twice — into the store by the
/// reconciler, and into the lifecycle event the run ledger reads. If those two
/// ever disagreed, the ledger would explain a failure differently from the
/// record behind it.
fn orphaned_subagent_reason(prior_status: OrchestrationTaskStatus) -> String {
    format!(
        "sub-agent orphaned by core restart (was `{}`)",
        task_status_label(prior_status)
    )
}

pub(crate) fn reconcile_orphaned_tasks_on_boot(workspace_dir: &Path) -> usize {
    let store = task_store_for_workspace(workspace_dir);

    // The sweep itself — which statuses are live, and which terminal state each
    // becomes — is TinyAgents'. What stays here is the reason a *sub-agent*
    // orphan carries, and the lifecycle event that finalizes OpenHuman's run
    // ledger afterwards.
    let report = reconcile_orphaned_tasks(
        store.as_ref(),
        OrchestrationTaskFilter::default().with_kind("sub_agent"),
        &|record| orphaned_subagent_reason(record.status),
    );

    if report.is_empty() {
        log::debug!(
            "[running_subagents] reconcile found no orphaned sub-agent tasks workspace_dir={}",
            workspace_dir.display()
        );
        return 0;
    }

    for task in report.settled() {
        let task_id = task.task_id.as_str().to_string();
        let prior = task_status_label(task.prior_status);
        let reason = orphaned_subagent_reason(task.prior_status);
        let parent_session = record_parent_session(&task.record)
            .unwrap_or_default()
            .to_string();
        let agent_id = record_agent_id(&task.record);
        // Reuse the 05.2 typed terminal lifecycle helper so the run ledger
        // finalizes exactly as it would for a live failure.
        super::subagent_events::publish_subagent_failed(
            parent_session,
            task_id.clone(),
            agent_id,
            reason,
        );
        log::info!(
            "[running_subagents] reconciled orphaned sub-agent task_id={} prior_status={} -> terminal",
            task_id,
            prior
        );
    }

    let reconciled = report.reconciled_count();
    log::info!(
        "[running_subagents] reconciled {reconciled} orphaned sub-agent task(s) on boot workspace_dir={} errors={}",
        workspace_dir.display(),
        report.error_count()
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
    let stores: Vec<Arc<dyn TaskStore>> = task_stores().values().unwrap_or_default();
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

#[derive(Clone)]
struct RunningSubagentMetadata {
    agent_id: String,
    subagent_session_id: Option<String>,
    workspace_dir: PathBuf,
    /// Parent chat thread that spawned this sub-agent, captured at registration.
    /// `None` for a headless spawn with no originating thread. Used to abort the
    /// sub-agent when its parent thread is deleted (see [`cancel_for_thread`]).
    parent_thread_id: Option<String>,
    run_queue: Arc<RunQueue>,
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

static REGISTRY: OnceLock<DetachedTaskRegistry<RunningSubagentMetadata, SubagentStatus>> =
    OnceLock::new();

fn registry() -> &'static DetachedTaskRegistry<RunningSubagentMetadata, SubagentStatus> {
    REGISTRY.get_or_init(|| {
        DetachedTaskRegistry::new(
            shared_steering_registry().clone(),
            REGISTRY_SOFT_CAP,
            SubagentStatus::is_terminal,
        )
    })
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

    let metadata = RunningSubagentMetadata {
        agent_id,
        subagent_session_id,
        workspace_dir,
        parent_thread_id,
        run_queue,
    };
    registry()
        .register(
            TaskId::new(task_id.clone()),
            parent_session,
            metadata,
            status,
            // Cooperative cancellation is flipped before the registry invokes
            // the hard abort. The child executor can adopt this token without
            // changing the registry/control API.
            CancellationToken::new(),
            abort,
        )
        .expect("duplicate detached sub-agent task id");
    log::debug!(
        "[running_subagents] registered task_id={} live_entries={}",
        task_id,
        registry()
            .len()
            .expect("detached task registry lock poisoned")
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
    let mut out: Vec<SubagentSnapshot> = registry()
        .snapshots(Some(parent_session))
        .expect("detached task registry lock poisoned")
        .into_iter()
        .map(|entry| {
            let status = match &entry.status {
                SubagentStatus::Running => "running",
                SubagentStatus::Completed { .. } => "completed",
                SubagentStatus::AwaitingUser { .. } => "awaiting_user",
                SubagentStatus::Failed { .. } => "failed",
            };
            SubagentSnapshot {
                agent_id: entry.metadata.agent_id,
                subagent_session_id: entry.metadata.subagent_session_id,
                task_id: entry.task_id.as_str().to_string(),
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

/// Most-recent durable sessions surfaced in the roster when they are not in
/// the live registry (cold boot / later turn). Bounds prompt growth on
/// threads with a long delegation history.
const DURABLE_ROSTER_CAP: usize = 12;
