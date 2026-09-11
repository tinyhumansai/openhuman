//! Per-session queue of *finished* detached background sub-agents
//! (`spawn_async_subagent`) awaiting delivery back into the chat.
//!
//! A detached sub-agent runs fire-and-forget; when it finishes, its result is
//! recorded here keyed by `parent_session`. The delivery subsystem
//! ([`super::background_delivery`]) drains the queue **when the session is
//! idle** (never mid-turn) and runs a single *system* turn on the parent chat
//! thread carrying every result ready at that moment — batched, with each one
//! tagged by its sub-agent process id. This module owns only the queue + the
//! notice formatting.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Mutex, OnceLock};

/// Terminal disposition of a finished background sub-agent. Drives distinct
/// rendering in [`build_batched_notice`] so a failed / awaiting-input async
/// sub-agent surfaces in chat as such instead of being dropped or mistaken for a
/// success (#4896).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum BackgroundAgentOutcome {
    /// Ran to a usable result (or partial progress framed as such).
    #[default]
    Completed,
    /// The child errored before producing a result.
    Failed,
    /// The child paused asking the user a question and was not continued.
    AwaitingInput,
}

/// One finished background sub-agent's deliverable result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CompletedBackgroundAgent {
    /// Spawn process id (`sub-…`) — the tag the agent uses to reference it.
    pub(crate) task_id: String,
    /// Sub-agent definition id (e.g. `researcher`).
    pub(crate) agent_id: String,
    /// The sub-agent's final output / summary.
    pub(crate) summary: String,
    /// Parent chat thread id to stream the delivery turn into (captured at
    /// spawn). `None` for a headless spawn with no originating thread.
    pub(crate) parent_thread_id: Option<String>,
    /// Terminal disposition — success, failure, or awaiting-user — so delivery
    /// can render failures/awaiting distinctly (#4896).
    pub(crate) outcome: BackgroundAgentOutcome,
}

/// Upper bound on the cancelled-thread tombstone set. A thread id is a one-shot
/// UUID, so only the *recently* cancelled threads can still be racing a late
/// completion; older tombstones are evicted in insertion order. 512 is far more
/// than the number of sub-agents that could realistically be mid-flight when a
/// batch of threads is deleted.
const CANCELLED_TOMBSTONE_CAP: usize = 512;

/// Upper bound on the collected-task tombstone set. A completion records within
/// seconds of the parent collecting it inline, so only recently collected task
/// ids can still be racing a late record; older tombstones are evicted in
/// insertion order.
const COLLECTED_TOMBSTONE_CAP: usize = 512;

/// Shared state behind a single mutex so the cancellation check in
/// [`record_completion`] is atomic against the tombstone+sweep in
/// [`discard_for_thread`] — otherwise the cooperative-abort race could enqueue a
/// completion for a thread that was just deleted (see issue #3711 review).
#[derive(Default)]
struct QueueState {
    /// Finished results awaiting idle delivery, keyed by `parent_session`.
    pending: HashMap<String, Vec<CompletedBackgroundAgent>>,
    /// Threads whose sub-agents were cancelled because the thread was
    /// deleted/purged. A completion that lands here *after* the discard sweep
    /// (Tokio `abort()` is cooperative — a task already past its last `.await`
    /// still runs to `record_completion`) is dropped instead of delivered into
    /// a thread that no longer exists.
    cancelled_threads: HashSet<String>,
    /// Insertion order for `cancelled_threads`, used to bound the set.
    cancelled_order: VecDeque<String>,
    /// Task ids the parent already collected inline via `wait_subagent` and will
    /// present in its own turn. A completion for a collected task is dropped by
    /// [`record_completion`] (closing the wait/record ordering race) and any
    /// already-queued entry is swept by [`mark_collected`], so background
    /// delivery never re-answers a result the master already surfaced (the
    /// duplicate-response bug).
    collected_tasks: HashSet<String>,
    /// Insertion order for `collected_tasks`, used to bound the set.
    collected_order: VecDeque<String>,
}

impl QueueState {
    /// Tombstone `thread_id` so any straggler completion for it is dropped.
    fn tombstone(&mut self, thread_id: &str) {
        if self.cancelled_threads.insert(thread_id.to_string()) {
            self.cancelled_order.push_back(thread_id.to_string());
            while self.cancelled_order.len() > CANCELLED_TOMBSTONE_CAP {
                if let Some(evicted) = self.cancelled_order.pop_front() {
                    self.cancelled_threads.remove(&evicted);
                }
            }
        }
    }

    /// Tombstone `task_id` so a completion that records after the parent
    /// collected it inline is dropped rather than delivered again.
    fn tombstone_collected(&mut self, task_id: &str) {
        if self.collected_tasks.insert(task_id.to_string()) {
            self.collected_order.push_back(task_id.to_string());
            while self.collected_order.len() > COLLECTED_TOMBSTONE_CAP {
                if let Some(evicted) = self.collected_order.pop_front() {
                    self.collected_tasks.remove(&evicted);
                }
            }
        }
    }
}

static QUEUE: OnceLock<Mutex<QueueState>> = OnceLock::new();

fn queue() -> &'static Mutex<QueueState> {
    QUEUE.get_or_init(|| Mutex::new(QueueState::default()))
}

/// Record a finished background sub-agent for later idle delivery, keyed by
/// `parent_session`. Idempotent on `task_id` within a session.
///
/// Drops the result outright if its parent thread has been tombstoned by
/// [`discard_for_thread`] — closing the race where a detached sub-agent finishes
/// (and records) concurrently with its parent thread being deleted.
pub(crate) fn record_completion(
    parent_session: impl Into<String>,
    task_id: impl Into<String>,
    agent_id: impl Into<String>,
    summary: impl Into<String>,
    parent_thread_id: Option<String>,
) {
    record_outcome(
        parent_session,
        task_id,
        agent_id,
        summary,
        parent_thread_id,
        BackgroundAgentOutcome::Completed,
    );
}

/// Record a finished background sub-agent carrying an explicit terminal
/// [`BackgroundAgentOutcome`]. This is the general enqueue behind
/// [`record_completion`] (success) and the [`record_failure`] /
/// [`record_awaiting_input`] framing helpers, so a failed or awaiting-input
/// async sub-agent is delivered back into chat too — not only successes (#4896).
/// Same tombstone / idempotency guarantees as [`record_completion`].
pub(crate) fn record_outcome(
    parent_session: impl Into<String>,
    task_id: impl Into<String>,
    agent_id: impl Into<String>,
    summary: impl Into<String>,
    parent_thread_id: Option<String>,
    outcome: BackgroundAgentOutcome,
) {
    let parent_session = parent_session.into();
    let entry = CompletedBackgroundAgent {
        task_id: task_id.into(),
        agent_id: agent_id.into(),
        summary: summary.into(),
        parent_thread_id,
        outcome,
    };
    let mut state = queue()
        .lock()
        .expect("background_completions queue poisoned");
    if let Some(thread_id) = entry.parent_thread_id.as_deref() {
        if state.cancelled_threads.contains(thread_id) {
            log::debug!(
                "[background_completions] dropping completion task_id={} for cancelled thread_id={}",
                entry.task_id,
                thread_id
            );
            return;
        }
    }
    // The parent already collected this result inline (`wait_subagent`) and
    // presents it in its own turn, so a background-delivery turn for it would
    // just re-answer the same thing. Drop it (closes the wait-before-record
    // race; the record-before-wait order is handled by the sweep in
    // `mark_collected`).
    if state.collected_tasks.contains(&entry.task_id) {
        log::debug!(
            "[background_completions] dropping completion task_id={} already collected inline",
            entry.task_id
        );
        return;
    }
    let pending = state.pending.entry(parent_session).or_default();
    if pending.iter().any(|c| c.task_id == entry.task_id) {
        return;
    }
    pending.push(entry);
}

/// Queue a **failed** async sub-agent for chat delivery (#4896). The summary is
/// framed with the `[SUBAGENT_FAILED]` envelope the parent agent is prompted to
/// relay, so the user learns the delegated task errored instead of the turn
/// silently finalizing on "Accepted". Enqueues via [`record_outcome`], so it
/// rides the same idle-gated `background_delivery` path as a success.
pub(crate) fn record_failure(
    parent_session: impl Into<String>,
    task_id: impl Into<String>,
    agent_id: impl Into<String>,
    error: &str,
    parent_thread_id: Option<String>,
) {
    let summary =
        format!("[SUBAGENT_FAILED] the async sub-agent errored before producing a result: {error}");
    record_outcome(
        parent_session,
        task_id,
        agent_id,
        summary,
        parent_thread_id,
        BackgroundAgentOutcome::Failed,
    );
}

/// Queue an **awaiting-user** async sub-agent for chat delivery (#4896). A
/// detached child that pauses to ask a question will not continue on its own, so
/// the framed `[SUBAGENT_NEEDS_INPUT]` notice is delivered back into chat for the
/// parent agent to relay to (or answer for) the user.
pub(crate) fn record_awaiting_input(
    parent_session: impl Into<String>,
    task_id: impl Into<String>,
    agent_id: impl Into<String>,
    question: &str,
    parent_thread_id: Option<String>,
) {
    let summary = format!(
        "[SUBAGENT_NEEDS_INPUT] the async sub-agent paused to ask the user a question and will \
         not continue on its own: {question}"
    );
    record_outcome(
        parent_session,
        task_id,
        agent_id,
        summary,
        parent_thread_id,
        BackgroundAgentOutcome::AwaitingInput,
    );
}

/// Is anything waiting to be delivered for this session? Cheap idle-loop check.
pub(crate) fn has_pending(parent_session: &str) -> bool {
    queue()
        .lock()
        .expect("background_completions queue poisoned")
        .pending
        .get(parent_session)
        .is_some_and(|v| !v.is_empty())
}

/// Number of results pending for a session.
pub(crate) fn pending_count(parent_session: &str) -> usize {
    queue()
        .lock()
        .expect("background_completions queue poisoned")
        .pending
        .get(parent_session)
        .map_or(0, Vec::len)
}

/// Drain **all** results currently ready for this session — the "batch
/// everything ready at that moment" step. Returns them in completion order and
/// clears them so they're never re-delivered.
pub(crate) fn take_pending(parent_session: &str) -> Vec<CompletedBackgroundAgent> {
    queue()
        .lock()
        .expect("background_completions queue poisoned")
        .pending
        .remove(parent_session)
        .unwrap_or_default()
}

/// Drop every queued completion whose `parent_thread_id` is `thread_id`, across
/// **all** sessions, and **tombstone** the thread so any straggler completion
/// that records *after* this sweep (the cooperative-abort race) is dropped by
/// [`record_completion`] rather than delivered into a thread that no longer
/// exists. Called when that thread is deleted. Returns the number of queued
/// completions removed.
pub(crate) fn discard_for_thread(thread_id: &str) -> usize {
    let mut state = queue()
        .lock()
        .expect("background_completions queue poisoned");
    state.tombstone(thread_id);
    let mut removed = 0;
    for pending in state.pending.values_mut() {
        let before = pending.len();
        pending.retain(|c| c.parent_thread_id.as_deref() != Some(thread_id));
        removed += before - pending.len();
    }
    // Drop now-empty session buckets so the map doesn't accumulate keys.
    state.pending.retain(|_, v| !v.is_empty());
    let sessions_left = state.pending.len();
    log::debug!(
        "[background_completions] discard_for_thread thread_id={} removed={} sessions_left={}",
        thread_id,
        removed,
        sessions_left
    );
    removed
}

/// Mark `task_id` as collected inline by the parent (via `wait_subagent`) so its
/// background completion is not independently delivered as a second, duplicate
/// answer. Tombstones the id — bounded — so a completion that records *after*
/// this call (the wait-before-record ordering) is dropped by
/// [`record_completion`], and sweeps any entry already queued for it across all
/// sessions (the record-before-wait ordering). Both orderings resolve
/// atomically under the single queue mutex. Returns whether a queued entry was
/// removed.
pub(crate) fn mark_collected(task_id: &str) -> bool {
    let mut state = queue()
        .lock()
        .expect("background_completions queue poisoned");
    state.tombstone_collected(task_id);
    let mut removed = false;
    for pending in state.pending.values_mut() {
        let before = pending.len();
        pending.retain(|c| c.task_id != task_id);
        removed |= pending.len() != before;
    }
    // Drop now-empty session buckets so the map doesn't accumulate keys.
    state.pending.retain(|_, v| !v.is_empty());
    log::debug!(
        "[background_completions] mark_collected task_id={task_id} removed_queued={removed}"
    );
    removed
}

/// Wipe every queued completion across all sessions. Called on a full thread
/// purge. Tombstones are left intact (the per-thread protection set by
/// [`discard_for_thread`]); the purge path tombstones each in-flight sub-agent's
/// thread before calling this, so stragglers are still dropped. Returns the
/// number of queued completions removed.
pub(crate) fn clear_all() -> usize {
    let mut state = queue()
        .lock()
        .expect("background_completions queue poisoned");
    let removed: usize = state.pending.values().map(Vec::len).sum();
    state.pending.clear();
    log::debug!("[background_completions] clear_all removed={}", removed);
    removed
}

/// The thread id to deliver a batch into — the first record that carries one.
pub(crate) fn batch_thread_id(completed: &[CompletedBackgroundAgent]) -> Option<String> {
    completed.iter().find_map(|c| c.parent_thread_id.clone())
}

/// Build the single batched, system-injected notice for a set of finished
/// background sub-agents. Each result is wrapped in a
/// `<background_agent_result id="…">` tag carrying its sub-agent process id, so
/// the agent can reference / present them individually. Returns `None` for an
/// empty batch.
pub(crate) fn build_batched_notice(completed: &[CompletedBackgroundAgent]) -> Option<String> {
    if completed.is_empty() {
        return None;
    }
    let n = completed.len();
    let mut out = String::new();
    out.push_str(&format!(
        "[{n} background sub-agent{} finished while you were busy. Review each result \
         below — including any that FAILED or NEED INPUT — and present what is relevant \
         to the user (never silently drop a failure or an awaiting-input pause). Each is \
         tagged with its sub-agent process id.]\n",
        if n == 1 { "" } else { "s" },
    ));
    for c in completed {
        // Distinct tag per terminal outcome so a failure / awaiting-input result
        // is not presented as a normal completion (#4896).
        let (tag, empty_fallback) = match c.outcome {
            BackgroundAgentOutcome::Completed => {
                ("background_agent_result", "(no output reported)")
            }
            BackgroundAgentOutcome::Failed => (
                "background_agent_failure",
                "(failed with no detail reported)",
            ),
            BackgroundAgentOutcome::AwaitingInput => (
                "background_agent_needs_input",
                "(the sub-agent paused awaiting user input)",
            ),
        };
        let summary = if c.summary.trim().is_empty() {
            empty_fallback
        } else {
            c.summary.trim()
        };
        out.push_str(&format!(
            "\n<{tag} id=\"{}\" agent=\"{}\">\n{}\n</{tag}>\n",
            c.task_id, c.agent_id, summary,
        ));
    }
    Some(out)
}

#[cfg(test)]
#[path = "background_completions_tests.rs"]
mod tests;
