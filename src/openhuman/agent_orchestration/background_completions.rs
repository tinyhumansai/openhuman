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
mod tests {
    use super::*;
    use std::sync::MutexGuard;

    /// Serializes every test that touches the global [`QUEUE`]. We reuse the
    /// crate-wide `TEST_ENV_LOCK` because `clear_all` is also reachable from the
    /// `threads::ops` purge test (which holds the same lock); a module-local
    /// mutex wouldn't prevent that cross-module race.
    fn test_guard() -> MutexGuard<'static, ()> {
        crate::openhuman::config::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    fn c(task: &str, agent: &str, summary: &str) -> CompletedBackgroundAgent {
        CompletedBackgroundAgent {
            task_id: task.into(),
            agent_id: agent.into(),
            summary: summary.into(),
            parent_thread_id: Some("thread-1".into()),
            outcome: BackgroundAgentOutcome::Completed,
        }
    }

    fn c_outcome(
        task: &str,
        agent: &str,
        summary: &str,
        outcome: BackgroundAgentOutcome,
    ) -> CompletedBackgroundAgent {
        CompletedBackgroundAgent {
            outcome,
            ..c(task, agent, summary)
        }
    }

    #[test]
    fn record_and_drain_is_session_scoped_and_batches() {
        let _guard = test_guard();
        let s = "sess-batch-A";
        record_completion(s, "sub-1", "researcher", "eiffel", Some("thread-A".into()));
        record_completion(s, "sub-2", "researcher", "liberty", Some("thread-A".into()));
        record_completion("sess-other", "sub-9", "researcher", "x", None);

        assert_eq!(pending_count(s), 2);
        assert!(has_pending(s));

        let drained = take_pending(s);
        assert_eq!(
            drained
                .iter()
                .map(|c| c.task_id.as_str())
                .collect::<Vec<_>>(),
            ["sub-1", "sub-2"]
        );
        assert_eq!(batch_thread_id(&drained).as_deref(), Some("thread-A"));
        assert!(!has_pending(s));
        assert_eq!(take_pending(s), vec![]);
        assert_eq!(pending_count("sess-other"), 1);
        take_pending("sess-other");
    }

    #[test]
    fn record_is_idempotent_on_task_id() {
        let _guard = test_guard();
        let s = "sess-dupe";
        record_completion(s, "sub-1", "researcher", "first", None);
        record_completion(s, "sub-1", "researcher", "second", None);
        assert_eq!(pending_count(s), 1);
        take_pending(s);
    }

    #[test]
    fn batched_notice_tags_each_with_process_id() {
        let notice = build_batched_notice(&[
            c("sub-abc", "researcher", "Eiffel Tower: built 1889 …"),
            c("sub-def", "researcher", "Colosseum: AD 70–80 …"),
        ])
        .expect("non-empty batch");

        assert!(notice.contains("2 background sub-agents finished"));
        assert!(notice.contains("<background_agent_result id=\"sub-abc\" agent=\"researcher\">"));
        assert!(notice.contains("Eiffel Tower: built 1889"));
        assert!(notice.contains("<background_agent_result id=\"sub-def\" agent=\"researcher\">"));
        assert!(notice.contains("</background_agent_result>"));
    }

    #[test]
    fn singular_wording_and_empty_summary_fallback() {
        let notice = build_batched_notice(&[c("sub-x", "researcher", "   ")]).unwrap();
        assert!(notice.contains("1 background sub-agent finished"));
        assert!(notice.contains("(no output reported)"));
    }

    #[test]
    fn empty_batch_is_none() {
        assert_eq!(build_batched_notice(&[]), None);
    }

    #[test]
    fn discard_for_thread_removes_matching_across_sessions() {
        let _guard = test_guard();
        // Two sessions, each with a completion for the doomed thread plus one
        // for a thread that must survive.
        record_completion(
            "sess-d1",
            "sub-a",
            "researcher",
            "x",
            Some("thread-DEL".into()),
        );
        record_completion(
            "sess-d1",
            "sub-b",
            "researcher",
            "y",
            Some("thread-KEEP".into()),
        );
        record_completion(
            "sess-d2",
            "sub-c",
            "researcher",
            "z",
            Some("thread-DEL".into()),
        );
        // Headless completion (no parent thread) must survive.
        record_completion("sess-d2", "sub-d", "researcher", "w", None);

        let removed = discard_for_thread("thread-DEL");
        assert_eq!(removed, 2, "both thread-DEL completions removed");

        // thread-KEEP survives in sess-d1; sess-d2 keeps only the headless one.
        assert_eq!(pending_count("sess-d1"), 1);
        let d1 = take_pending("sess-d1");
        assert_eq!(d1[0].task_id, "sub-b");

        assert_eq!(pending_count("sess-d2"), 1);
        let d2 = take_pending("sess-d2");
        assert_eq!(d2[0].task_id, "sub-d");

        // Idempotent: nothing left to discard.
        assert_eq!(discard_for_thread("thread-DEL"), 0);
    }

    #[test]
    fn record_after_discard_is_dropped_by_tombstone() {
        let _guard = test_guard();
        // Deleting the thread tombstones it...
        discard_for_thread("thread-race");
        // ...so a straggler completion that records *after* the sweep (the
        // cooperative-abort race) is dropped rather than queued.
        record_completion(
            "sess-race",
            "sub-late",
            "researcher",
            "stale",
            Some("thread-race".into()),
        );
        assert_eq!(
            pending_count("sess-race"),
            0,
            "late completion for a cancelled thread must be dropped"
        );
        // A completion for a different, live thread still records normally.
        record_completion(
            "sess-race",
            "sub-ok",
            "researcher",
            "fresh",
            Some("thread-live-race".into()),
        );
        assert_eq!(pending_count("sess-race"), 1);
        take_pending("sess-race");
    }

    #[test]
    fn clear_all_empties_the_queue() {
        let _guard = test_guard();
        record_completion("sess-c1", "sub-1", "researcher", "a", Some("t1".into()));
        record_completion("sess-c2", "sub-2", "researcher", "b", None);

        let removed = clear_all();
        assert!(
            removed >= 2,
            "clear_all should report at least the two just queued, got {removed}"
        );
        assert!(!has_pending("sess-c1"));
        assert!(!has_pending("sess-c2"));
        assert_eq!(clear_all(), 0);
    }

    #[test]
    fn mark_collected_sweeps_the_queued_entry() {
        let _guard = test_guard();
        let s = "sess-mc-sweep";
        record_completion(s, "mc-sub-1", "researcher", "collected", None);
        record_completion(s, "mc-sub-2", "researcher", "keep", None);

        // The parent collected sub-1 inline, so it must not be delivered again;
        // sub-2 (never waited on) survives for normal idle delivery.
        assert!(mark_collected("mc-sub-1"), "swept the queued entry");
        assert_eq!(pending_count(s), 1);
        let drained = take_pending(s);
        assert_eq!(drained[0].task_id, "mc-sub-2");
    }

    #[test]
    fn record_after_mark_collected_is_dropped_by_tombstone() {
        let _guard = test_guard();
        // Collecting inline tombstones the task id...
        assert!(
            !mark_collected("mc-late"),
            "nothing queued yet, so nothing swept"
        );
        // ...so a completion that records *after* (the wait-before-record order)
        // is dropped rather than queued for a duplicate delivery turn.
        record_completion("sess-mc-race", "mc-late", "researcher", "stale", None);
        assert_eq!(
            pending_count("sess-mc-race"),
            0,
            "a completion collected inline must not be re-delivered"
        );
    }

    #[test]
    fn mark_collected_is_task_scoped() {
        let _guard = test_guard();
        let s = "sess-mc-scope";
        // Only the collected task is suppressed; an un-waited sibling still
        // surfaces (the genuinely-later fire-and-forget feature is preserved).
        mark_collected("mc-scope-1");
        record_completion(s, "mc-scope-2", "researcher", "later", None);
        assert_eq!(pending_count(s), 1);
        assert!(has_pending(s));
        take_pending(s);
    }

    #[test]
    fn collected_tombstone_is_bounded() {
        let _guard = test_guard();
        for i in 0..(COLLECTED_TOMBSTONE_CAP + 50) {
            mark_collected(&format!("mc-bound-{i}"));
        }
        let len = queue()
            .lock()
            .expect("queue poisoned")
            .collected_tasks
            .len();
        assert!(
            len <= COLLECTED_TOMBSTONE_CAP,
            "collected tombstone must stay bounded, got {len}"
        );
    }

    // ── #4896: failure / awaiting-user delivery ─────────────────────────────

    #[test]
    fn record_failure_queues_a_framed_failure_for_delivery() {
        let _guard = test_guard();
        let s = "sess-fail";
        record_failure(
            s,
            "sub-f",
            "researcher",
            "provider 500: inference failed",
            Some("thread-F".into()),
        );
        // The failure rode the SAME queue successes use → it will be delivered.
        assert_eq!(pending_count(s), 1);
        let drained = take_pending(s);
        assert_eq!(drained[0].outcome, BackgroundAgentOutcome::Failed);
        assert!(drained[0].summary.starts_with("[SUBAGENT_FAILED]"));
        assert!(drained[0]
            .summary
            .contains("provider 500: inference failed"));
    }

    #[test]
    fn record_awaiting_input_queues_a_framed_needs_input_for_delivery() {
        let _guard = test_guard();
        let s = "sess-await";
        record_awaiting_input(
            s,
            "sub-a",
            "researcher",
            "Which repo should I open the PR against?",
            Some("thread-A".into()),
        );
        assert_eq!(pending_count(s), 1);
        let drained = take_pending(s);
        assert_eq!(drained[0].outcome, BackgroundAgentOutcome::AwaitingInput);
        assert!(drained[0].summary.starts_with("[SUBAGENT_NEEDS_INPUT]"));
        assert!(drained[0].summary.contains("Which repo"));
    }

    #[test]
    fn notice_renders_failure_and_awaiting_with_distinct_tags() {
        let notice = build_batched_notice(&[
            c_outcome(
                "sub-ok",
                "researcher",
                "all good",
                BackgroundAgentOutcome::Completed,
            ),
            c_outcome(
                "sub-bad",
                "researcher",
                "[SUBAGENT_FAILED] boom",
                BackgroundAgentOutcome::Failed,
            ),
            c_outcome(
                "sub-ask",
                "researcher",
                "[SUBAGENT_NEEDS_INPUT] which repo?",
                BackgroundAgentOutcome::AwaitingInput,
            ),
        ])
        .expect("non-empty batch");

        // The header now tells the agent to surface failures / awaiting-input.
        assert!(notice.contains("FAILED or NEED INPUT"));
        // Each outcome renders under its own tag so a failure is not presented
        // as a normal completion.
        assert!(notice.contains("<background_agent_result id=\"sub-ok\" agent=\"researcher\">"));
        assert!(notice.contains("<background_agent_failure id=\"sub-bad\" agent=\"researcher\">"));
        assert!(notice.contains("[SUBAGENT_FAILED] boom"));
        assert!(
            notice.contains("<background_agent_needs_input id=\"sub-ask\" agent=\"researcher\">")
        );
        assert!(notice.contains("[SUBAGENT_NEEDS_INPUT] which repo?"));
    }

    #[test]
    fn empty_summary_fallback_is_outcome_specific() {
        let failed = build_batched_notice(&[c_outcome(
            "sub-e",
            "r",
            "   ",
            BackgroundAgentOutcome::Failed,
        )])
        .unwrap();
        assert!(failed.contains("(failed with no detail reported)"));

        let awaiting = build_batched_notice(&[c_outcome(
            "sub-e",
            "r",
            "",
            BackgroundAgentOutcome::AwaitingInput,
        )])
        .unwrap();
        assert!(awaiting.contains("(the sub-agent paused awaiting user input)"));
    }

    #[test]
    fn record_outcome_preserves_the_outcome_through_a_drain() {
        // Guards the requeue path (background_delivery::requeue re-enqueues via
        // record_outcome): a failed batch that fails delivery must not be
        // downgraded to a success on retry.
        let _guard = test_guard();
        let s = "sess-preserve";
        record_outcome(
            s,
            "sub-p",
            "researcher",
            "[SUBAGENT_FAILED] x",
            None,
            BackgroundAgentOutcome::Failed,
        );
        let drained = take_pending(s);
        assert_eq!(drained[0].outcome, BackgroundAgentOutcome::Failed);
    }
}
