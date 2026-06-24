//! Registry of in-flight async sub-agents that can be **steered** mid-run.
//!
//! `spawn_async_subagent` runs a child as a detached `tokio` task. On its own
//! that task is opaque: the parent gets a `task_id` back but has no channel into
//! the running loop and no way to collect the result inline. This registry
//! closes both gaps.
//!
//! Each running async sub-agent registers, keyed by its `task_id`, with:
//! - an `Arc<RunQueue>` — the same steering channel the inner `run_turn_engine`
//!   drains at iteration boundaries, so `steer_subagent` can inject a message;
//! - a `watch::Receiver<SubagentStatus>` — so `wait_subagent` can block until the
//!   child reaches a terminal status;
//! - an `AbortHandle` — kept for a future `close_agent` tool.
//!
//! Ownership is enforced: only the spawning parent (matched by `parent_session`)
//! may steer or wait on a given sub-agent. Terminal entries are pruned on `wait`,
//! and swept on `register` only once the table passes a soft cap, so it can't
//! grow unbounded if a parent never waits (the Codex "spawn-slot leak" failure
//! mode — openai/codex#18335).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use tokio::sync::watch;
use tokio::task::AbortHandle;

use crate::openhuman::agent::harness::run_queue::{QueueMode, QueuedMessage, RunQueue};

/// Terminal/transient state of a running async sub-agent, published by the
/// spawner's background task and observed by `wait_subagent`.
#[derive(Debug, Clone)]
pub enum SubagentStatus {
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
    pub fn is_terminal(&self) -> bool {
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
    status: watch::Receiver<SubagentStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentResumeRef {
    pub task_id: String,
    pub agent_id: String,
    pub subagent_session_id: Option<String>,
}

/// Soft cap on registry size. Terminal entries are only swept when the table
/// grows past this, so the common case (a handful of live sub-agents) never
/// evicts a still-uncollected terminal result out from under a `wait`/`steer`.
const REGISTRY_SOFT_CAP: usize = 256;

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
pub fn status_channel() -> (
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
pub fn register(
    task_id: String,
    agent_id: String,
    parent_session: String,
    subagent_session_id: Option<String>,
    workspace_dir: PathBuf,
    parent_thread_id: Option<String>,
    run_queue: Arc<RunQueue>,
    abort: AbortHandle,
    status: watch::Receiver<SubagentStatus>,
) {
    let entry = RunningSubagentEntry {
        agent_id,
        parent_session,
        subagent_session_id,
        workspace_dir,
        parent_thread_id,
        run_queue,
        abort,
        status,
    };
    let mut map = registry().lock().expect("running_subagents mutex poisoned");
    if map.len() >= REGISTRY_SOFT_CAP {
        // Only under genuine pressure: sweep collected/terminal entries so the
        // table can't grow without bound when a parent never waits (the Codex
        // spawn-slot leak). Live (Running) entries are always retained.
        map.retain(|_, e| !e.status.borrow().is_terminal());
    }
    map.insert(task_id.clone(), entry);
    log::debug!(
        "[running_subagents] registered task_id={} live_entries={}",
        task_id,
        map.len()
    );
}

/// Resolve a durable `subagent_session_id` to the currently-running transient
/// `task_id`, enforcing parent-session ownership.
pub fn task_id_for_session(
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

pub fn resume_ref_for_task(
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

/// Inject a message into a running sub-agent's steering queue. The child's
/// `run_turn_engine` drains it at the next iteration boundary.
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
pub async fn steer_control(task_id: &str, text: String, mode: QueueMode) -> Result<(), SteerError> {
    let run_queue = {
        let map = registry().lock().expect("running_subagents mutex poisoned");
        let entry = map.get(task_id).ok_or(SteerError::Unknown)?;
        if entry.status.borrow().is_terminal() {
            return Err(SteerError::AlreadyDone);
        }
        entry.run_queue.clone()
    };

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
pub enum WaitError {
    Unknown,
    NotOwned,
}

/// Result of waiting on a sub-agent.
#[derive(Debug)]
pub enum WaitOutcome {
    /// The sub-agent reached a terminal status (entry pruned).
    Terminal(SubagentStatus),
    /// The timeout elapsed first; the entry is left intact so the parent can
    /// wait again. Carries the latest (non-terminal) status snapshot.
    TimedOut(SubagentStatus),
}

/// Block until `task_id` reaches a terminal status or `timeout` elapses.
pub async fn wait(
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

/// Metadata captured when a sub-agent is cancelled, so the caller can surface
/// the cancellation back in the parent chat (record a "cancelled" completion
/// for idle-gated delivery).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CancelledSubagent {
    pub agent_id: String,
    pub parent_session: String,
    pub subagent_session_id: Option<String>,
    pub workspace_dir: PathBuf,
    pub parent_thread_id: Option<String>,
}

/// Abort and drop the sub-agent with `task_id`, returning its metadata so the
/// caller can deliver a "cancelled" notice into the parent chat. Returns `None`
/// if no such sub-agent is registered (already finished, or unknown id).
///
/// Unlike [`close`], this is keyed by `task_id` alone with no parent-session
/// ownership check — it backs the user-facing "Cancel" affordance, and the
/// desktop user owns every sub-agent in their own core.
pub fn cancel_by_task(task_id: &str) -> Option<CancelledSubagent> {
    let mut map = registry().lock().expect("running_subagents mutex poisoned");
    let entry = map.remove(task_id)?;
    entry.abort.abort();
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

pub fn cancel_by_session(
    subagent_session_id: &str,
    parent_session: &str,
) -> Option<CancelledSubagent> {
    let task_id = task_id_for_session(subagent_session_id, parent_session).ok()?;
    cancel_by_task(&task_id)
}

/// Abort a running sub-agent and drop its registry entry. Kept for a future
/// `close_agent` tool; the abort handle is stored at spawn time.
pub fn close(task_id: &str, parent_session: &str) -> bool {
    let mut map = registry().lock().expect("running_subagents mutex poisoned");
    match map.get(task_id) {
        Some(entry) if entry.parent_session == parent_session => {
            entry.abort.abort();
            map.remove(task_id);
            true
        }
        _ => false,
    }
}

/// Abort and drop every running sub-agent whose parent chat thread is
/// `thread_id`. Called when that thread is deleted so detached children don't
/// keep running (and later try to deliver) against a thread that no longer
/// exists. Returns the number of sub-agents cancelled.
pub fn cancel_for_thread(thread_id: &str) -> usize {
    let mut map = registry().lock().expect("running_subagents mutex poisoned");
    let to_cancel: Vec<String> = map
        .iter()
        .filter(|(_, e)| e.parent_thread_id.as_deref() == Some(thread_id))
        .map(|(id, _)| id.clone())
        .collect();
    for id in &to_cancel {
        if let Some(entry) = map.remove(id) {
            entry.abort.abort();
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
pub fn cancel_all() -> Vec<String> {
    let mut map = registry().lock().expect("running_subagents mutex poisoned");
    let count = map.len();
    let mut thread_ids: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for (_, entry) in map.drain() {
        entry.abort.abort();
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
            std::env::temp_dir(),
            parent_thread_id.map(Into::into),
            rq,
            dummy_abort(),
            rx,
        );
        tx
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
            Some("subsess-1".into()),
            std::env::temp_dir(),
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
    async fn resume_ref_for_task_includes_resume_fields_and_enforces_ownership() {
        let _guard = test_guard();
        let (tx, rx) = status_channel();
        register(
            "task-resume".into(),
            "researcher".into(),
            "session-owner".into(),
            Some("subsess-resume".into()),
            std::env::temp_dir(),
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
            Some("subsess-live".into()),
            std::env::temp_dir(),
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
            Some("subsess-live".into()),
            std::env::temp_dir(),
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

        let meta = cancel_by_task("task-cbt").expect("known task should cancel");
        assert_eq!(meta.agent_id, "researcher");
        assert_eq!(meta.parent_session, "session-Z");
        assert_eq!(meta.parent_thread_id.as_deref(), Some("thread-cbt"));

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
