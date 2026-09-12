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
    assert!(notice.contains("<background_agent_needs_input id=\"sub-ask\" agent=\"researcher\">"));
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
