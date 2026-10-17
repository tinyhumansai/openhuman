use super::*;
async fn leased(root: &Path, thread: &str, phase: Phase) -> Work {
    let work = store::start(
        root,
        thread,
        "Deliver actual feature".into(),
        vec!["Works end to end".into()],
        None,
    )
    .await
    .unwrap();
    store::update(root, thread, &work.id, |w| {
        w.phase = phase;
        w.lease = Some("lease".into());
        Ok(())
    })
    .await
    .unwrap()
}
fn verified() -> Observed {
    Observed {
        successful: 2,
        shell: 1,
        failed: 0,
        fingerprint: 42,
    }
}
#[tokio::test]
async fn finish_requires_separate_review_and_observed_checks() {
    let tmp = tempfile::tempdir().unwrap();
    let work = leased(tmp.path(), "verify", Phase::Reviewing).await;
    store::update(tmp.path(), "verify", &work.id, |w| {
        w.finish_requested = true;
        w.evidence = vec!["integration test passed".into()];
        Ok(())
    })
    .await
    .unwrap();
    let result = settle(tmp.path(), &work, "lease", &Observed::default(), None)
        .await
        .unwrap();
    assert_eq!(result.phase, Phase::Working);
    let work = store::update(tmp.path(), "verify", &work.id, |w| {
        w.phase = Phase::Reviewing;
        w.lease = Some("review2".into());
        w.finish_requested = true;
        Ok(())
    })
    .await
    .unwrap();
    let result = settle(tmp.path(), &work, "review2", &verified(), None)
        .await
        .unwrap();
    assert_eq!(result.phase, Phase::Complete);
    assert_eq!(
        crate::agent::goals::store::get(tmp.path(), "verify")
            .await
            .unwrap()
            .unwrap()
            .status
            .as_str(),
        "complete"
    );
}
#[tokio::test]
async fn implementation_cannot_finish_in_the_same_slice() {
    let tmp = tempfile::tempdir().unwrap();
    let work = leased(tmp.path(), "early", Phase::Working).await;
    store::update(tmp.path(), "early", &work.id, |w| {
        w.phase = Phase::Reviewing;
        w.finish_requested = true;
        w.evidence = vec!["claim".into()];
        Ok(())
    })
    .await
    .unwrap();
    assert_ne!(
        settle(tmp.path(), &work, "lease", &verified(), None)
            .await
            .unwrap()
            .phase,
        Phase::Complete
    );
}
#[tokio::test]
async fn stop_and_stale_lease_cannot_be_undone_by_late_success() {
    let tmp = tempfile::tempdir().unwrap();
    let work = leased(tmp.path(), "late", Phase::Reviewing).await;
    let result = settle(tmp.path(), &work, "stale", &verified(), None)
        .await
        .unwrap();
    assert_eq!(result.attempts, 0);
    store::pause(tmp.path(), "late", None).await.unwrap();
    assert_eq!(
        settle(tmp.path(), &work, "lease", &verified(), None)
            .await
            .unwrap()
            .phase,
        Phase::Paused
    );
}
#[tokio::test]
async fn transient_errors_back_off_and_auth_errors_block() {
    let tmp = tempfile::tempdir().unwrap();
    let work = leased(tmp.path(), "retry", Phase::Working).await;
    let before = now_ms();
    let result = settle(
        tmp.path(),
        &work,
        "lease",
        &Observed::default(),
        Some(("429".into(), true, Some(120000))),
    )
    .await
    .unwrap();
    assert!(result.runnable());
    assert!(result.next_run_ms >= before + 120000);
    let work = store::update(tmp.path(), "retry", &work.id, |w| {
        w.lease = Some("lease2".into());
        Ok(())
    })
    .await
    .unwrap();
    assert_eq!(
        settle(
            tmp.path(),
            &work,
            "lease2",
            &Observed::default(),
            Some(("invalid credential".into(), false, None))
        )
        .await
        .unwrap()
        .phase,
        Phase::Blocked
    );
}
#[tokio::test]
async fn repeated_empty_turns_stop_but_many_productive_turns_do_not() {
    let tmp = tempfile::tempdir().unwrap();
    let mut work = leased(tmp.path(), "progress", Phase::Working).await;
    for i in 0..210 {
        work = store::update(tmp.path(), "progress", &work.id, |w| {
            w.lease = Some("lease".into());
            Ok(())
        })
        .await
        .unwrap();
        let mut observed = verified();
        observed.fingerprint = i;
        work = settle(tmp.path(), &work, "lease", &observed, None)
            .await
            .unwrap();
        assert!(work.runnable());
    }
    for _ in 0..3 {
        work = store::update(tmp.path(), "progress", &work.id, |w| {
            w.lease = Some("lease".into());
            Ok(())
        })
        .await
        .unwrap();
        work = settle(tmp.path(), &work, "lease", &Observed::default(), None)
            .await
            .unwrap();
    }
    assert_eq!(work.phase, Phase::Blocked);
}
#[test]
fn only_real_successful_check_activity_passes_gate() {
    let guard = observation::Guard::new("observed-test");
    observation::record(
        "observed-test",
        "autonomous_work",
        true,
        &None,
        "claimed done",
    );
    assert!(!observation::verified(&guard.snapshot()));
    observation::record("observed-test", "file_read", true, &None, "actual source");
    observation::record("observed-test", "shell", true, &None, "tests pass");
    assert!(observation::verified(&guard.snapshot()));
    observation::record("observed-test", "shell", false, &None, "failed test");
    assert!(!observation::verified(&guard.snapshot()));
}

#[tokio::test]
async fn budget_limit_between_admission_and_finish_wins() {
    let tmp = tempfile::tempdir().unwrap();
    let work = leased(tmp.path(), "budget-finish", Phase::Reviewing).await;
    crate::agent::goals::store::set(tmp.path(), "budget-finish", &work.objective, Some(1))
        .await
        .unwrap();
    crate::agent::goals::runtime::account_turn_against_goal(
        tmp.path(),
        Some("budget-finish"),
        2,
        0,
        1,
    )
    .await;
    store::update(tmp.path(), "budget-finish", &work.id, |w| {
        w.finish_requested = true;
        w.evidence = vec!["claim".into()];
        Ok(())
    })
    .await
    .unwrap();
    assert_eq!(
        settle(tmp.path(), &work, "lease", &verified(), None)
            .await
            .unwrap()
            .phase,
        Phase::Paused
    );
}

#[tokio::test]
async fn new_worker_report_invalidates_review_without_resuming_a_stopped_task() {
    let tmp = tempfile::tempdir().unwrap();
    let work = leased(tmp.path(), "worker-review", Phase::Reviewing).await;
    assert!(
        store::worker_report(
            tmp.path(),
            "worker-review",
            "Worker says done; inspect actual diff"
        )
        .await
        .unwrap()
    );
    let result = settle(tmp.path(), &work, "lease", &verified(), None)
        .await
        .unwrap();
    assert_eq!(result.phase, Phase::Working);
    assert_eq!(result.worker_revision, 1);
    store::pause(tmp.path(), "worker-review", None)
        .await
        .unwrap();
    store::worker_report(tmp.path(), "worker-review", "Late report")
        .await
        .unwrap();
    assert_eq!(
        store::get(tmp.path(), "worker-review")
            .await
            .unwrap()
            .unwrap()
            .phase,
        Phase::Paused
    );
}

#[tokio::test]
async fn protocol_errors_get_automatic_repair_but_never_false_completion() {
    let tmp = tempfile::tempdir().unwrap();
    let mut work = leased(tmp.path(), "protocol", Phase::Working).await;
    for attempt in 1..=3 {
        work = store::update(tmp.path(), "protocol", &work.id, |w| {
            w.lease = Some("lease".into());
            Ok(())
        })
        .await
        .unwrap();
        work = settle(
            tmp.path(),
            &work,
            "lease",
            &Observed::default(),
            Some(("tool_call_recovery_exhausted".into(), true, None)),
        )
        .await
        .unwrap();
        assert_eq!(
            work.phase,
            if attempt < 3 {
                Phase::Working
            } else {
                Phase::Blocked
            }
        );
    }
}

#[test]
fn discovery_and_stale_execution_are_not_verification() {
    let guard = observation::Guard::new("stale-verification");
    observation::record(
        "stale-verification",
        "tool_search",
        true,
        &None,
        "tools found",
    );
    assert_eq!(guard.snapshot().successful, 0);
    observation::record("stale-verification", "file_read", true, &None, "source");
    observation::record("stale-verification", "shell", true, &None, "tests pass");
    assert!(observation::verified(&guard.snapshot()));
    for edit in ["file_write", "apply_patch"] {
        observation::record("stale-verification", edit, true, &None, "changed code");
        assert!(!observation::verified(&guard.snapshot()));
        observation::record("stale-verification", "file_read", true, &None, "new source");
        assert!(!observation::verified(&guard.snapshot()));
        observation::record("stale-verification", "shell", true, &None, "rerun passed");
        assert!(observation::verified(&guard.snapshot()));
    }
    observation::record(
        "stale-verification",
        "shell",
        false,
        &None,
        "Command failed (exit code 1)\ncorrect-looking stdout",
    );
    assert!(!observation::verified(&guard.snapshot()));
}

#[tokio::test]
async fn stagnant_slices_receive_different_recovery_before_blocking() {
    let tmp = tempfile::tempdir().unwrap();
    let mut work = leased(tmp.path(), "adaptive-recovery", Phase::Working).await;
    for step in 1..=3 {
        work = store::update(tmp.path(), &work.thread_id, &work.id, |w| {
            w.lease = Some("lease".into());
            Ok(())
        })
        .await
        .unwrap();
        work = settle(tmp.path(), &work, "lease", &Observed::default(), None)
            .await
            .unwrap();
        let saved = store::get(tmp.path(), &work.thread_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved.stagnant_turns, step);
        match step {
            1 => assert!(saved.prompt().contains("RECOVERY:")),
            2 => assert!(saved.prompt().contains("REPLAN:")),
            _ => assert_eq!(saved.phase, Phase::Blocked),
        }
    }
}
