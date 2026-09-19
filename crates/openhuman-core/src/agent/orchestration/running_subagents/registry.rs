//! The in-process registry of running async sub-agents: registration,
//! per-task metadata, and the terminal/transient [`SubagentStatus`] each entry
//! carries.
//!
//! Each running async sub-agent registers here (via [`register`]), keyed by
//! its `task_id`, with:
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
//! TinyAgents owns the process-local watch/cancel/abort/steering mechanics.
//! OpenHuman retains product metadata, durable task-store projection, and the
//! legacy `RunQueue` steering fallback. Ownership is enforced by parent session;
//! terminal entries are pruned on `wait` and swept at the registry soft cap.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use tokio::sync::watch;
use tokio::task::AbortHandle;

use crate::agent::tinyagents::host::steering::shared_steering_registry;
use tinyagents_graph::orchestration::DetachedTaskRegistry;
use tinyagents_harness::ids::TaskId;
use tinyagents_harness::run_queue::RunQueue;
use tinyagents_harness::CancellationToken;

use super::task_ledger::record_spawned;

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
    pub(crate) fn is_terminal(&self) -> bool {
        !matches!(self, SubagentStatus::Running)
    }
}

#[derive(Clone)]
pub(crate) struct RunningSubagentMetadata {
    pub(crate) agent_id: String,
    pub(crate) subagent_session_id: Option<String>,
    pub(crate) workspace_dir: PathBuf,
    /// Parent chat thread that spawned this sub-agent, captured at registration.
    /// `None` for a headless spawn with no originating thread. Used to abort the
    /// sub-agent when its parent thread is deleted (see [`super::cancel::cancel_for_thread`]).
    pub(crate) parent_thread_id: Option<String>,
    pub(crate) run_queue: Arc<RunQueue<crate::agent::queued_turn::QueuedTurn>>,
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
pub(crate) const DETACHED_LEDGER_TIMEOUT_MS: u64 = 120_000;

static REGISTRY: OnceLock<DetachedTaskRegistry<RunningSubagentMetadata, SubagentStatus>> =
    OnceLock::new();

pub(crate) fn registry() -> &'static DetachedTaskRegistry<RunningSubagentMetadata, SubagentStatus> {
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
#[allow(clippy::too_many_arguments)]
pub(crate) fn register(
    task_id: String,
    agent_id: String,
    parent_session: String,
    session_parent_prefix: Option<String>,
    subagent_session_id: Option<String>,
    workspace_dir: PathBuf,
    parent_thread_id: Option<String>,
    run_queue: Arc<RunQueue<crate::agent::queued_turn::QueuedTurn>>,
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
/// terminal status is recorded as a failure, matching [`super::wait::wait`].
fn spawn_status_watcher(
    task_id: String,
    workspace_dir: PathBuf,
    mut status: watch::Receiver<SubagentStatus>,
) {
    tokio::spawn(async move {
        loop {
            let snapshot = status.borrow_and_update().clone();
            if snapshot.is_terminal() {
                super::task_ledger::record_status(&workspace_dir, &task_id, &snapshot);
                break;
            }
            if status.changed().await.is_err() {
                super::task_ledger::record_status(
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
