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

use std::collections::HashMap;
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
    #[allow(dead_code)]
    agent_id: String,
    parent_session: String,
    run_queue: Arc<RunQueue>,
    abort: AbortHandle,
    status: watch::Receiver<SubagentStatus>,
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
    run_queue: Arc<RunQueue>,
    abort: AbortHandle,
    status: watch::Receiver<SubagentStatus>,
) {
    let entry = RunningSubagentEntry {
        agent_id,
        parent_session,
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
        let (tx, rx) = status_channel();
        register(
            task_id.into(),
            "researcher".into(),
            parent_session.into(),
            rq,
            dummy_abort(),
            rx,
        );
        tx
    }

    #[tokio::test]
    async fn steer_pushes_into_the_subagent_queue() {
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
}
