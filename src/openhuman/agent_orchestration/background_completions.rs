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

/// One finished background sub-agent's deliverable result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletedBackgroundAgent {
    /// Spawn process id (`sub-…`) — the tag the agent uses to reference it.
    pub task_id: String,
    /// Sub-agent definition id (e.g. `researcher`).
    pub agent_id: String,
    /// The sub-agent's final output / summary.
    pub summary: String,
    /// Parent chat thread id to stream the delivery turn into (captured at
    /// spawn). `None` for a headless spawn with no originating thread.
    pub parent_thread_id: Option<String>,
}

/// Upper bound on the cancelled-thread tombstone set. A thread id is a one-shot
/// UUID, so only the *recently* cancelled threads can still be racing a late
/// completion; older tombstones are evicted in insertion order. 512 is far more
/// than the number of sub-agents that could realistically be mid-flight when a
/// batch of threads is deleted.
const CANCELLED_TOMBSTONE_CAP: usize = 512;

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
pub fn record_completion(
    parent_session: impl Into<String>,
    task_id: impl Into<String>,
    agent_id: impl Into<String>,
    summary: impl Into<String>,
    parent_thread_id: Option<String>,
) {
    let parent_session = parent_session.into();
    let entry = CompletedBackgroundAgent {
        task_id: task_id.into(),
        agent_id: agent_id.into(),
        summary: summary.into(),
        parent_thread_id,
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
    let pending = state.pending.entry(parent_session).or_default();
    if pending.iter().any(|c| c.task_id == entry.task_id) {
        return;
    }
    pending.push(entry);
}

/// Is anything waiting to be delivered for this session? Cheap idle-loop check.
pub fn has_pending(parent_session: &str) -> bool {
    queue()
        .lock()
        .expect("background_completions queue poisoned")
        .pending
        .get(parent_session)
        .is_some_and(|v| !v.is_empty())
}

/// Number of results pending for a session.
pub fn pending_count(parent_session: &str) -> usize {
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
pub fn take_pending(parent_session: &str) -> Vec<CompletedBackgroundAgent> {
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
pub fn discard_for_thread(thread_id: &str) -> usize {
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

/// Wipe every queued completion across all sessions. Called on a full thread
/// purge. Tombstones are left intact (the per-thread protection set by
/// [`discard_for_thread`]); the purge path tombstones each in-flight sub-agent's
/// thread before calling this, so stragglers are still dropped. Returns the
/// number of queued completions removed.
pub fn clear_all() -> usize {
    let mut state = queue()
        .lock()
        .expect("background_completions queue poisoned");
    let removed: usize = state.pending.values().map(Vec::len).sum();
    state.pending.clear();
    log::debug!("[background_completions] clear_all removed={}", removed);
    removed
}

/// The thread id to deliver a batch into — the first record that carries one.
pub fn batch_thread_id(completed: &[CompletedBackgroundAgent]) -> Option<String> {
    completed.iter().find_map(|c| c.parent_thread_id.clone())
}

/// Build the single batched, system-injected notice for a set of finished
/// background sub-agents. Each result is wrapped in a
/// `<background_agent_result id="…">` tag carrying its sub-agent process id, so
/// the agent can reference / present them individually. Returns `None` for an
/// empty batch.
pub fn build_batched_notice(completed: &[CompletedBackgroundAgent]) -> Option<String> {
    if completed.is_empty() {
        return None;
    }
    let n = completed.len();
    let mut out = String::new();
    out.push_str(&format!(
        "[{n} background sub-agent{} finished while you were busy. Review each result \
         below and present what is relevant to the user. Each is tagged with its \
         sub-agent process id.]\n",
        if n == 1 { "" } else { "s" },
    ));
    for c in completed {
        let summary = if c.summary.trim().is_empty() {
            "(no output reported)"
        } else {
            c.summary.trim()
        };
        out.push_str(&format!(
            "\n<background_agent_result id=\"{}\" agent=\"{}\">\n{}\n</background_agent_result>\n",
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
}
