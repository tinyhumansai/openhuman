use super::*;
use crate::openhuman::agent::orchestration::background_completions::record_completion;

#[test]
fn plan_drains_ready_batch_when_idle() {
    let s = "bd-ready";
    record_completion(s, "sub-1", "researcher", "alpha", Some("thread-9".into()));
    record_completion(s, "sub-2", "researcher", "beta", Some("thread-9".into()));

    let batch = plan_delivery(s).expect("plans a delivery");
    assert_eq!(batch.len(), 2);
    assert_eq!(
        background_completions::batch_thread_id(&batch).as_deref(),
        Some("thread-9")
    );
    let notice = background_completions::build_batched_notice(&batch).unwrap();
    assert!(notice.contains("sub-1") && notice.contains("sub-2"));
    assert!(!background_completions::has_pending(s)); // drained
}

#[test]
fn plan_skips_when_busy_and_leaves_queue_intact() {
    let s = "bd-busy";
    record_completion(s, "sub-1", "researcher", "x", Some("t".into()));
    busy().lock().expect("busy").insert(s.to_string());

    assert!(plan_delivery(s).is_none());
    assert!(background_completions::has_pending(s)); // NOT drained while busy

    busy().lock().expect("busy").remove(s);
    let _ = background_completions::take_pending(s); // cleanup
}

#[test]
fn plan_none_when_nothing_pending() {
    assert!(plan_delivery("bd-empty-unique").is_none());
}

#[test]
fn headless_batch_has_no_thread_so_caller_drops_it() {
    let s = "bd-headless";
    record_completion(s, "sub-1", "researcher", "x", None);
    let batch = plan_delivery(s).expect("batch present");
    // No originating thread → batch_thread_id is None, so try_deliver drops it.
    assert!(background_completions::batch_thread_id(&batch).is_none());
}

#[test]
fn requeue_restores_a_failed_batch() {
    let s = "bd-requeue";
    record_completion(s, "sub-1", "researcher", "alpha", Some("t".into()));
    let batch = plan_delivery(s).expect("batch");
    assert!(!background_completions::has_pending(s)); // drained
    requeue(s, batch);
    assert!(background_completions::has_pending(s)); // restored for retry
    let _ = background_completions::take_pending(s); // cleanup
}

#[test]
fn interleave_recheck_requeues_when_user_turn_starts_after_drain() {
    // Mirrors try_deliver's M1 guard: a user turn can start between
    // plan_delivery draining the batch and the awaited system turn. The
    // re-check must requeue the drained batch rather than stream concurrently.
    let s = "bd-interleave";
    record_completion(s, "sub-1", "researcher", "alpha", Some("t".into()));

    let batch = plan_delivery(s).expect("batch drained");
    assert!(!background_completions::has_pending(s)); // drained

    // User turn starts after the drain, before the (would-be) await.
    busy().lock().expect("busy").insert(s.to_string());
    if is_busy(s) {
        requeue(s, batch); // the guard's action
    }
    assert!(background_completions::has_pending(s)); // preserved for next drain

    busy().lock().expect("busy").remove(s);
    let _ = background_completions::take_pending(s); // cleanup
}

#[tokio::test]
async fn handler_tracks_busy_across_turn_and_error_events() {
    let h = BackgroundDeliveryHandler;
    let sid = "bd-turn".to_string();

    h.handle(&DomainEvent::AgentTurnStarted {
        session_id: sid.clone(),
        channel: "test".into(),
    })
    .await;
    assert!(is_busy(&sid));

    h.handle(&DomainEvent::AgentTurnCompleted {
        session_id: sid.clone(),
        text_chars: 0,
        iterations: 0,
    })
    .await;
    assert!(!is_busy(&sid));

    // A failed turn (AgentError) must also clear busy so delivery isn't stuck.
    busy().lock().expect("busy").insert(sid.clone());
    h.handle(&DomainEvent::AgentError {
        session_id: sid.clone(),
        message: "boom".into(),
        recoverable: true,
    })
    .await;
    assert!(!is_busy(&sid));
}

#[tokio::test(start_paused = true)]
async fn every_subagent_terminal_event_schedules_a_drain() {
    // #4896 regression: EVERY subagent terminal event must schedule a drain
    // for the parent — not just `SubagentCompleted`. Before the fix,
    // `SubagentFailed` / `SubagentAwaitingUser` fell through to `_ => {}`, so
    // a failure/pause recorded after the parent turn went idle was never
    // delivered. Prove behaviour, not just acceptance: queue a headless
    // result (no thread → drains without a delivery sink) per session, fire
    // the event, advance past the debounce, and assert the pending item was
    // consumed. The paused clock elapses the debounce with no wall-clock wait.
    let h = BackgroundDeliveryHandler;

    background_completions::record_completion("bd-term-completed", "t", "a", "s", None);
    background_completions::record_completion("bd-term-failed", "t", "a", "s", None);
    background_completions::record_completion("bd-term-awaiting", "t", "a", "s", None);

    h.handle(&DomainEvent::SubagentCompleted {
        parent_session: "bd-term-completed".into(),
        task_id: "t".into(),
        agent_id: "a".into(),
        elapsed_ms: 0,
        output_chars: 0,
        iterations: 0,
    })
    .await;
    h.handle(&DomainEvent::SubagentFailed {
        parent_session: "bd-term-failed".into(),
        task_id: "t".into(),
        agent_id: "a".into(),
        error: "boom".into(),
    })
    .await;
    h.handle(&DomainEvent::SubagentAwaitingUser {
        parent_session: "bd-term-awaiting".into(),
        task_id: "t".into(),
        agent_id: "a".into(),
        question: "?".into(),
    })
    .await;

    // Advance the virtual clock past the debounce so every scheduled drain
    // runs; the headless `try_deliver` completes synchronously (no sink).
    tokio::time::sleep(DEBOUNCE + Duration::from_millis(50)).await;
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }

    assert!(
        !background_completions::has_pending("bd-term-completed"),
        "SubagentCompleted must schedule a drain that consumes the pending result"
    );
    assert!(
        !background_completions::has_pending("bd-term-failed"),
        "SubagentFailed must schedule a drain (regression #4896)"
    );
    assert!(
        !background_completions::has_pending("bd-term-awaiting"),
        "SubagentAwaitingUser must schedule a drain (regression #4896)"
    );
}
