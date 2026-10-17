//! The in-process registry of running async sub-agents: registration,
//! per-task metadata, and the terminal/transient [`DetachedSubagentStatus`] each entry
//! carries.
//!
//! Each running async sub-agent registers here (via [`register`]), keyed by
//! its `task_id`, with:
//! - an `Arc<RunQueue>` — the same steering channel the steering forwarder in
//!   `TurnSpec::run` drains mid-turn, so `steer_subagent` can
//!   inject a message when no crate-native steering handle is registered;
//! - a TinyAgents `SteeringHandle` in the process-local
//!   `SteeringRegistry` while the child TinyAgents run is active, so
//!   steer/collect controls can deliver directly to the crate queue;
//! - a `watch::Receiver<DetachedSubagentStatus>` — so `wait_subagent` can block until the
//!   child reaches a terminal status;
//! - an `AbortHandle` — used by `subagent_cancel`/`close_subagent` paths to stop
//!   detached work.
//!
//! TinyAgents owns the process-local watch/cancel/abort/steering mechanics.
//! OpenHuman retains product metadata, durable task-store projection, and the
//! legacy `RunQueue` steering fallback. Ownership is enforced by captured profile
//! and parent session;
//! terminal entries are pruned on `wait` and swept at the registry soft cap.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use tokio::sync::watch;
use tokio::task::AbortHandle;

use crate::agent::tinyagents::host::steering::shared_steering_registry;
use tinyagents_harness::ids::TaskId;
use tinyagents_harness::run_queue::RunQueue;
use tinyagents_harness::CancellationToken;
use tinyagents_tasks::DetachedTaskRegistry;

use super::task_ledger::{record_spawned, task_store_for_workspace};
use tinyagents_orchestration::subagent::{
    spawn_status_watcher, DetachedSubagentStatus, SubagentIdentity, WaitError,
};

#[derive(Clone)]
pub(crate) struct RunningSubagentMetadata {
    /// Original profile authority, independent of caller-chosen session IDs.
    pub(crate) profile: Option<String>,
    /// Public/durable session ID; the registry owner uses an encoded key.
    pub(crate) parent_session: String,
    pub(crate) agent_id: String,
    pub(crate) subagent_session_id: Option<String>,
    pub(crate) workspace_dir: PathBuf,
    /// Parent chat thread that spawned this sub-agent, captured at registration.
    /// `None` for a headless spawn with no originating thread. Used to abort the
    /// sub-agent when its parent thread is deleted (see [`super::cancel::cancel_for_thread`]).
    pub(crate) parent_thread_id: Option<String>,
    pub(crate) run_queue: Arc<RunQueue<crate::agent::queued_turn::QueuedTurn>>,
}

/// Resolve the caller's profile without falling back to the operator in SaaS.
fn caller_profile(ctx: &crate::core::runtime::DispatchCtx) -> Result<Option<String>, WaitError> {
    let profile = ctx.tenant().map_err(|_| WaitError::NotOwned)?.profile;
    if crate::core::runtime::is_saas() && profile.is_none() {
        return Err(WaitError::NotOwned);
    }
    Ok(profile)
}

/// A refused detached spawn cannot be reported as accepted work.
#[derive(Debug, thiserror::Error)]
pub(crate) enum RegistrationError {
    #[error("missing or mismatched tenant scope")]
    Scope,
    #[error("detached task registration failed: {0}")]
    Registry(#[source] tinyagents_harness::error::TinyAgentsError),
}

/// Check the actual caller before any reusable-session or worker-thread write.
pub(crate) fn admit_spawn(
    ctx: &crate::core::runtime::DispatchCtx,
    workspace: &std::path::Path,
) -> Result<Option<String>, RegistrationError> {
    admit_workspace(ctx, workspace).map_err(|_| RegistrationError::Scope)?;
    caller_profile(ctx).map_err(|_| RegistrationError::Scope)
}

/// Host ownership adapter over TinyAgents' opaque owner string.
pub(super) fn owner_key(
    ctx: &crate::core::runtime::DispatchCtx,
    parent_session: &str,
) -> Result<String, WaitError> {
    let profile = caller_profile(ctx)?;
    Ok(crate::core::runtime::tenant::tenant_key(
        &crate::core::runtime::tenant::Tenant {
            profile,
            agent: None,
        },
        parent_session,
    ))
}

/// Public user controls may reach any of their profile's parents; desktop
/// controls retain the process user's existing authority over all workers.
pub(super) fn control_admits(
    ctx: &crate::core::runtime::DispatchCtx,
    metadata: &RunningSubagentMetadata,
) -> bool {
    !crate::core::runtime::is_saas()
        || caller_profile(ctx).is_ok_and(|profile| profile == metadata.profile)
}

/// Durable fallback paths must stay in the caller's captured workspace.
pub(super) fn admit_workspace(
    ctx: &crate::core::runtime::DispatchCtx,
    workspace: &std::path::Path,
) -> Result<(), WaitError> {
    if !crate::core::runtime::is_saas() {
        return Ok(());
    }
    caller_profile(ctx)?;
    let context = ctx.resolved().ok_or(WaitError::NotOwned)?;
    if context.workspace_dir().map_err(|_| WaitError::NotOwned)? != workspace {
        return Err(WaitError::NotOwned);
    }
    Ok(())
}

impl SubagentIdentity for RunningSubagentMetadata {
    fn agent_id(&self) -> &str {
        &self.agent_id
    }
    fn subagent_session_id(&self) -> Option<&str> {
        self.subagent_session_id.as_deref()
    }
}

/// Soft cap on registry size. Terminal entries are only swept when the table
/// grows past this, so the common case (a handful of live sub-agents) never
/// evicts a still-uncollected terminal result out from under a `wait`/`steer`.
const REGISTRY_SOFT_CAP: usize = 256;
static REGISTRY: OnceLock<DetachedTaskRegistry<RunningSubagentMetadata, DetachedSubagentStatus>> =
    OnceLock::new();

pub(crate) fn registry(
) -> &'static DetachedTaskRegistry<RunningSubagentMetadata, DetachedSubagentStatus> {
    REGISTRY.get_or_init(|| {
        DetachedTaskRegistry::new(
            shared_steering_registry().clone(),
            REGISTRY_SOFT_CAP,
            DetachedSubagentStatus::is_terminal,
        )
    })
}

/// Create the status channel a spawner threads into [`register`].
///
/// The spawner moves the [`watch::Sender`] into its detached task and `send`s a
/// terminal [`DetachedSubagentStatus`] on completion. Dropping the sender (e.g. a
/// panicked/aborted task) closes the channel, which `wait_subagent` surfaces as
/// a failure rather than hanging.
pub(crate) fn status_channel() -> (
    watch::Sender<DetachedSubagentStatus>,
    watch::Receiver<DetachedSubagentStatus>,
) {
    watch::channel(DetachedSubagentStatus::Running)
}

/// Register a running async sub-agent so it can be steered and waited on.
///
/// Call this *after* `tokio::spawn` so the [`AbortHandle`] is available; the
/// task owns the matching [`watch::Sender`] from [`status_channel`]. Once the
/// table passes [`REGISTRY_SOFT_CAP`], registration sweeps already-terminal
/// entries so it stays bounded even if a parent never calls `wait_subagent`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn register(
    ctx: &crate::core::runtime::DispatchCtx,
    task_id: String,
    agent_id: String,
    parent_session: String,
    session_parent_prefix: Option<String>,
    subagent_session_id: Option<String>,
    workspace_dir: PathBuf,
    parent_thread_id: Option<String>,
    run_queue: Arc<RunQueue<crate::agent::queued_turn::QueuedTurn>>,
    abort: AbortHandle,
    status: watch::Receiver<DetachedSubagentStatus>,
) -> Result<(), RegistrationError> {
    let profile = match admit_spawn(ctx, &workspace_dir) {
        Ok(profile) => profile,
        Err(error) => {
            abort.abort();
            log::warn!("[running_subagents] refusing registration without a SaaS profile");
            return Err(error);
        }
    };
    let owner = crate::core::runtime::tenant::tenant_key(
        &crate::core::runtime::tenant::Tenant {
            profile: profile.clone(),
            agent: None,
        },
        &parent_session,
    );
    if let Some(thread_id) = parent_thread_id.as_deref() {
        if crate::agent::orchestration::background_completions::mark_stopped_task_if_thread_stopped(
            ctx,
            &workspace_dir,
            thread_id,
            &task_id,
        ) {
            // Stop landed after the child was spawned but before this registry
            // entry existed. The completion is tombstoned above; abort promptly
            // so the detached work does not keep consuming resources either.
            abort.abort();
            log::debug!(
                "[running_subagents] aborted late registration task_id={} thread_id={}",
                task_id,
                thread_id
            );
        }
        // First spawn in a workspace this process has not scanned: redeliver any
        // completions a previous process finished there but never delivered.
        crate::agent::orchestration::background_delivery::recover_on_boot(ctx, &workspace_dir);
    }

    let metadata = RunningSubagentMetadata {
        profile,
        parent_session: parent_session.clone(),
        agent_id: agent_id.clone(),
        subagent_session_id: subagent_session_id.clone(),
        workspace_dir: workspace_dir.clone(),
        parent_thread_id: parent_thread_id.clone(),
        run_queue,
    };
    registry()
        .register(
            TaskId::new(task_id.clone()),
            owner,
            metadata,
            status.clone(),
            // Cooperative cancellation is flipped before the registry invokes
            // the hard abort. The child executor can adopt this token without
            // changing the registry/control API.
            CancellationToken::new(),
            abort.clone(),
        )
        .map_err(|error| {
            abort.abort();
            RegistrationError::Registry(error)
        })?;
    // Failed admission must not leave a Running lifecycle record or watcher.
    record_spawned(
        &task_id,
        &agent_id,
        &parent_session,
        session_parent_prefix.as_deref(),
        subagent_session_id.as_deref(),
        &workspace_dir,
        parent_thread_id.as_deref(),
    );
    spawn_status_watcher(
        task_store_for_workspace(&workspace_dir),
        task_id.clone(),
        status,
    );
    log::debug!(
        "[running_subagents] registered task_id={} live_entries={}",
        task_id,
        registry()
            .len()
            .expect("detached task registry lock poisoned")
    );
    Ok(())
}
