use super::*;

#[test]
fn unknown_task_send_and_abort_are_noops() {
    let mgr = MedullaTaskManager::new();
    // Neither should panic when the task id is unknown.
    mgr.steer_task(payloads::TaskSend {
        task_id: "nope".into(),
        input: "hi".into(),
    });
    mgr.abort_task(payloads::TaskAbort {
        task_id: "nope".into(),
    });
}

#[test]
fn duplicate_task_registration_is_rejected() {
    let mgr = Arc::new(MedullaTaskManager::new());
    // Manually seed a running task to simulate an in-flight run, then prove
    // a second registration under the same id is ignored.
    let abort = CancellationToken::new();
    let (steer_tx, _rx) = mpsc::unbounded_channel();
    mgr.tasks
        .lock()
        .insert("dup".to_string(), RunningTask { abort, steer_tx });
    assert!(mgr.tasks.lock().contains_key("dup"));
    // A second start_task for "dup" must not overwrite / spawn.
    mgr.start_task(payloads::TaskRun {
        task_id: "dup".into(),
        cycle_id: "c".into(),
        session_id: None,
        instruction: "x".into(),
        agent_id: None,
        timeout_ms: 0,
    });
    assert_eq!(mgr.tasks.lock().len(), 1);
}

#[test]
fn session_key_scopes_transcript_by_session_id() {
    // Same agent id + distinct session ids => distinct transcript keys, so
    // two medulla sessions can't collide onto one shared transcript.
    let a = medulla_session_key("orchestrator", "sess-abc");
    let b = medulla_session_key("orchestrator", "sess-xyz");
    assert_ne!(a, b);
    assert_eq!(a, "orchestrator_sess-abc");
    assert!(a.starts_with("orchestrator_"));
    // Overlong session ids are truncated on a char boundary.
    let long = "x".repeat(100);
    let key = medulla_session_key("orchestrator", &long);
    assert_eq!(key, format!("orchestrator_{}", "x".repeat(32)));
}

#[test]
fn an_undecodable_probe_answers_not_ready_when_it_names_itself() {
    let raw = serde_json::json!({ "probeId": "p-1", "agentId": 7 });
    let result = unparsed_capabilities_result(&raw, "invalid type: integer")
        .expect("a probe that names itself is answerable");
    assert_eq!(result.probe_id, "p-1");
    // `ready` + `readyReason` are the two fields the backend's allowlist
    // keeps; an answer outside them sanitizes to an empty bag, which the
    // probe treats as no answer at all.
    assert_eq!(result.capabilities["ready"], false);
    assert!(result.capabilities["readyReason"]
        .as_str()
        .expect("a readable reason")
        .contains("invalid type: integer"));
}

#[test]
fn an_undecodable_probe_without_a_probe_id_is_unanswerable() {
    // Nothing to correlate, so nothing can be answered — and the socket read
    // loop must not panic over it either.
    let raw = serde_json::json!({ "agentId": "orchestrator" });
    assert!(unparsed_capabilities_result(&raw, "missing field `probeId`").is_none());
    reject_unparsed_capabilities_request(&raw, "missing field `probeId`");
}

#[test]
fn remaining_budget_reports_time_left_and_exhaustion() {
    let now = Instant::now();
    // No deadline configured => unbounded.
    assert_eq!(remaining_budget(None, now), Ok(None));
    // Deadline in the future => remaining time until it.
    let future = now + Duration::from_secs(10);
    match remaining_budget(Some(future), now) {
        Ok(Some(d)) => assert!(d <= Duration::from_secs(10) && d > Duration::from_secs(9)),
        other => panic!("expected some remaining budget, got {other:?}"),
    }
    // Deadline already reached / passed => exhausted.
    assert_eq!(remaining_budget(Some(now), now), Err(()));
    assert_eq!(
        remaining_budget(Some(now), now + Duration::from_secs(1)),
        Err(())
    );
}
