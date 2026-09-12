
/// Build the ambient `[active_subagents]` block prepended to a parent's turn
/// context. Returns `None` when the parent owns no sub-agents at all, so the
/// block only appears when it is actionable — turns for agents that never
/// spawn are untouched. Mirrors the thread-goal `[active_goal]` block: it
/// rides the per-turn context (not the cached system-prompt prefix), so it
/// reflects live status every turn.
///
/// The roster merges two sources:
/// 1. the in-memory registry (live async workers spawned this process), and
/// 2. the durable per-workspace `subagent_sessions` store — workers from
///    EARLIER turns / process lifetimes. Without this second source a
///    cold-booted parent had no idea its previous sub-agents existed and
///    would re-delegate from scratch instead of resuming by
///    `subagent_session_id` (the "fresh context from day 0" bug).
pub(crate) fn active_subagents_context_block(
    parent_session: &str,
    workspace_dir: &std::path::Path,
) -> Option<String> {
    let workers = snapshot_for_parent(parent_session);

    // Durable sessions not already represented by a live registry entry.
    let live_session_ids: std::collections::HashSet<String> = workers
        .iter()
        .filter_map(|w| w.subagent_session_id.clone())
        .collect();
    let store = crate::openhuman::agent::orchestration::subagent_sessions::SubagentSessionStore {
        workspace_dir: workspace_dir.to_path_buf(),
    };
    let durable: Vec<_> =
        match crate::openhuman::agent::orchestration::subagent_sessions::list_for_parent(
            &store,
            parent_session,
            None,
        ) {
            Ok(sessions) => sessions
                .into_iter()
                .filter(|s| {
                    use crate::openhuman::agent::orchestration::subagent_sessions::DurableSubagentStatus;
                    s.status != DurableSubagentStatus::Closed
                        && !live_session_ids.contains(&s.subagent_session_id)
                })
                .take(DURABLE_ROSTER_CAP)
                .collect(),
            Err(err) => {
                log::warn!(
                    "[running_subagents] durable roster load failed parent_session={parent_session} error={err}"
                );
                Vec::new()
            }
        };

    if workers.is_empty() && durable.is_empty() {
        return None;
    }
    let mut block = format!(
        "[active_subagents]\n\
         You have {} sub-agent worker(s) for this conversation (live and/or from earlier \
         turns). This is your authoritative roster — trust it over memory. Track each by \
         subagent_session_id; use wait_subagent to collect a `completed` one, steer_subagent \
         to redirect a `running` one, continue_subagent to answer an `awaiting_user` one or \
         to RESUME an `idle` one with a follow-up (it keeps its full prior context — do NOT \
         re-delegate the same task from scratch), close_subagent when done, and \
         list_subagents to re-enumerate. Never fabricate a result for a worker still running \
         or one that has failed.\n",
        workers.len() + durable.len()
    );
    for w in &workers {
        let session = w.subagent_session_id.as_deref().unwrap_or("(none)");
        block.push_str(&format!(
            "- {} · session={} · task={} · status={}\n",
            w.agent_id, session, w.task_id, w.status
        ));
    }
    for s in &durable {
        use crate::openhuman::agent::orchestration::subagent_sessions::DurableSubagentStatus;
        let status = match s.status {
            DurableSubagentStatus::Running => "running",
            DurableSubagentStatus::Idle => "idle",
            DurableSubagentStatus::AwaitingUser => "awaiting_user",
            DurableSubagentStatus::Failed => "failed",
            DurableSubagentStatus::Closed => "closed",
        };
        let task = s.current_task_id.as_deref().unwrap_or("(none)");
        block.push_str(&format!(
            "- {} · session={} · task={} · status={} · about: {}\n",
            s.agent_id, s.subagent_session_id, task, status, s.task_title
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
    let mut saw_unowned = false;
    let mut owned_terminal: Option<String> = None;
    for snapshot in registry()
        .snapshots(None)
        .expect("detached task registry lock poisoned")
        .into_iter()
        .filter(|snapshot| {
            snapshot.metadata.subagent_session_id.as_deref() == Some(subagent_session_id)
        })
    {
        if snapshot.owner_id != parent_session {
            saw_unowned = true;
            continue;
        }
        let task_id = snapshot.task_id.as_str().to_string();
        if !snapshot.status.is_terminal() {
            return Ok(task_id);
        }
        owned_terminal.get_or_insert(task_id);
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
    matches.sort_by_key(|item| std::cmp::Reverse(item.updated_at));

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
    let snapshot = registry()
        .snapshot(&TaskId::new(task_id), parent_session)
        .map_err(wait_error_from_registry)?;
    Ok(SubagentResumeRef {
        task_id: task_id.to_string(),
        agent_id: snapshot.metadata.agent_id,
        subagent_session_id: snapshot.metadata.subagent_session_id,
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

fn send_registered_steering(
    handle: &tinyagents_harness::steering::SteeringHandle,
    text: String,
    mode: QueueMode,
) -> bool {
    let Some(command) = steering_command_for_mode(mode, text) else {
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
    let handle = registry()
        .steering_handle(&TaskId::new(task_id), parent_session)
        .map_err(steer_directive_error_from_registry)?;
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

/// Inject a message into a running sub-agent. Prefer the crate-native
/// TinyAgents steering registry when the child run has registered its live
/// handle, and fall back to the OpenHuman `RunQueue` compatibility path.
pub async fn steer(
    task_id: &str,
    parent_session: &str,
    text: String,
    mode: QueueMode,
) -> Result<(), SteerError> {
    let task_id_key = TaskId::new(task_id);
    let snapshot = registry()
        .snapshot(&task_id_key, parent_session)
        .map_err(steer_error_from_registry)?;
    if snapshot.status.is_terminal() {
        return Err(SteerError::AlreadyDone);
    }

    let steered_via_registry = registry()
        .steering_handle(&task_id_key, parent_session)
        .map(|handle| send_registered_steering(&handle, text.clone(), mode))
        .unwrap_or(false);
    if steered_via_registry {
        log::info!(
            "[running_subagents] steered task_id={} mode={} via=tinyagents_registry",
            task_id,
            mode
        );
        return Ok(());
    }

    snapshot
        .metadata
        .run_queue
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
    let task_id_key = TaskId::new(task_id);
    let snapshot = registry()
        .snapshot_trusted(&task_id_key)
        .map_err(steer_error_from_registry)?;
    if snapshot.status.is_terminal() {
        return Err(SteerError::AlreadyDone);
    }

    let steered_via_registry = registry()
        .steering_handle_trusted(&task_id_key)
        .map(|handle| send_registered_steering(&handle, text.clone(), mode))
        .unwrap_or(false);
    if steered_via_registry {
        log::info!(
            "[running_subagents] control_steered task_id={} mode={} via=tinyagents_registry",
            task_id,
            mode
        );
        return Ok(());
    }

    snapshot
        .metadata
        .run_queue
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

fn wait_error_from_registry(error: DetachedTaskRegistryError) -> WaitError {
    match error {
        DetachedTaskRegistryError::NotOwned => WaitError::NotOwned,
        _ => WaitError::Unknown,
    }
}

fn steer_error_from_registry(error: DetachedTaskRegistryError) -> SteerError {
    match error {
        DetachedTaskRegistryError::NotOwned => SteerError::NotOwned,
        DetachedTaskRegistryError::AlreadyDone => SteerError::AlreadyDone,
        _ => SteerError::Unknown,
    }
}

fn steer_directive_error_from_registry(error: DetachedTaskRegistryError) -> SteerDirectiveError {
    match error {
        DetachedTaskRegistryError::NotOwned => SteerDirectiveError::NotOwned,
        DetachedTaskRegistryError::AlreadyDone => SteerDirectiveError::AlreadyDone,
        DetachedTaskRegistryError::NoSteeringHandle => SteerDirectiveError::NoRegisteredHandle,
        _ => SteerDirectiveError::Unknown,
    }
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
    match registry()
        .wait(&TaskId::new(task_id), parent_session, timeout)
        .await
    {
        Ok(DetachedTaskWaitOutcome::Terminal(status)) => Ok(WaitOutcome::Terminal(status)),
        Ok(DetachedTaskWaitOutcome::TimedOut(status)) => Ok(WaitOutcome::TimedOut(status)),
        Err(DetachedTaskRegistryError::StatusChannelClosed) => {
            Ok(WaitOutcome::Terminal(SubagentStatus::Failed {
                error: "sub-agent task ended without reporting a result".to_string(),
            }))
        }
        Err(error) => Err(wait_error_from_registry(error)),
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
    let cancelled = registry().cancel_trusted(&TaskId::new(task_id)).ok()?;
    let metadata = cancelled.metadata;
    record_cancelled(&metadata.workspace_dir, task_id);
    log::debug!(
        "[running_subagents] cancel_by_task task_id={} agent_id={} parent_thread_id={:?} live_entries={}",
        task_id,
        metadata.agent_id,
        metadata.parent_thread_id,
        registry()
            .len()
            .expect("detached task registry lock poisoned")
    );
    Some(CancelledSubagent {
        agent_id: metadata.agent_id,
        parent_session: cancelled.owner_id,
        subagent_session_id: metadata.subagent_session_id,
        workspace_dir: metadata.workspace_dir,
        parent_thread_id: metadata.parent_thread_id,
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
    let cancelled = registry()
        .cancel_where(|metadata| metadata.parent_thread_id.as_deref() == Some(thread_id))
        .expect("detached task registry lock poisoned");
    for entry in &cancelled {
        record_cancelled(&entry.metadata.workspace_dir, entry.task_id.as_str());
    }
    let count = cancelled.len();
    log::debug!(
        "[running_subagents] cancel_for_thread thread_id={} cancelled={} live_entries={}",
        thread_id,
        count,
        registry()
            .len()
            .expect("detached task registry lock poisoned")
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
    let cancelled = registry()
        .cancel_all()
        .expect("detached task registry lock poisoned");
    let count = cancelled.len();
    let mut thread_ids: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for entry in cancelled {
        record_cancelled(&entry.metadata.workspace_dir, entry.task_id.as_str());
        if let Some(thread_id) = entry.metadata.parent_thread_id {
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
    let _ = registry().cancel_trusted(&TaskId::new(task_id));
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
