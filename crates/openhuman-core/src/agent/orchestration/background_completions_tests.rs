use super::*;
use tinyagents_tasks::CompletionStatus;
use tokio::sync::MutexGuard;

/// Serializes every test that touches the process-wide router registry. We reuse
/// the crate-wide `TEST_ENV_LOCK` because `clear_all` sweeps every open
/// workspace and is also reachable from the `threads::ops` purge test (which
/// holds the same lock); a module-local mutex wouldn't prevent that cross-module
/// race.
fn test_guard() -> MutexGuard<'static, ()> {
    crate::config::TEST_ENV_LOCK.blocking_lock()
}

fn workspace() -> TestWorkspace {
    TestWorkspace::new()
}

fn record(ws: &Path, session: &str, task: &str, summary: &str, thread: Option<&str>) {
    futures::executor::block_on(record_completion(
        ws,
        session,
        task,
        "researcher",
        summary,
        thread.map(str::to_owned),
    ));
}

fn pending_ids(ws: &Path, thread: &str) -> Vec<String> {
    pending_for(ws, thread)
        .into_iter()
        .map(|r| r.task_id)
        .collect()
}

#[test]
fn record_is_thread_scoped_and_batches_in_completion_order() {
    let _guard = test_guard();
    let ws = workspace();
    let w = ws.path();
    record(w, "sess-A", "sub-1", "eiffel", Some("thread-A"));
    record(w, "sess-A", "sub-2", "liberty", Some("thread-A"));
    record(w, "sess-B", "sub-9", "x", Some("thread-B"));

    assert_eq!(pending_ids(w, "thread-A"), ["sub-1", "sub-2"]);
    assert_eq!(pending_ids(w, "thread-B"), ["sub-9"]);

    let router = router_for_workspace(w);
    let claimed = router.claim_pending("thread-A", usize::MAX).unwrap();
    assert_eq!(claimed.len(), 2, "everything ready is claimed as one batch");
    assert_eq!(claimed[0].result.text, "eiffel");
    assert!(router
        .claim_pending("thread-A", usize::MAX)
        .unwrap()
        .is_empty());
}

#[test]
fn record_is_idempotent_on_task_id() {
    let _guard = test_guard();
    let ws = workspace();
    let w = ws.path();
    record(w, "sess-dupe", "sub-1", "first", Some("thread-dupe"));
    record(w, "sess-dupe", "sub-1", "second", Some("thread-dupe"));
    let pending = pending_for(w, "thread-dupe");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].result.text, "first", "the first record wins");
}

#[test]
fn headless_completion_is_dropped() {
    let _guard = test_guard();
    let ws = workspace();
    record(ws.path(), "sess-headless", "sub-1", "x", None);
    assert!(router_for_workspace(ws.path())
        .claim_pending("", usize::MAX)
        .unwrap()
        .is_empty());
    assert!(recover_pending_threads(ws.path()).is_empty());
}

#[test]
fn the_session_is_mapped_to_its_thread_for_the_idle_gate() {
    let _guard = test_guard();
    let ws = workspace();
    record(ws.path(), "sess-map", "sub-1", "x", Some("thread-map"));
    assert_eq!(
        thread_for_session("sess-map").as_deref(),
        Some("thread-map")
    );
    // A web-channel session id carries its thread in the JSON body.
    let web = r#"{"client_id":"c","thread_id":"thread-json"}"#;
    assert_eq!(thread_for_session(web).as_deref(), Some("thread-json"));
    assert_eq!(thread_for_session("cron:job-1"), None);
}

#[test]
fn discard_for_thread_removes_matching_and_drops_stragglers() {
    let _guard = test_guard();
    let ws = workspace();
    let w = ws.path();
    record(w, "sess-d1", "sub-a", "x", Some("thread-DEL"));
    record(w, "sess-d1", "sub-b", "y", Some("thread-KEEP"));
    record(w, "sess-d2", "sub-c", "z", Some("thread-DEL"));

    assert_eq!(discard_for_thread(w, "thread-DEL"), 2);
    assert!(pending_ids(w, "thread-DEL").is_empty());
    assert_eq!(pending_ids(w, "thread-KEEP"), ["sub-b"]);
}

#[test]
fn record_after_discard_is_dropped_by_the_cancelled_parent() {
    let _guard = test_guard();
    let ws = workspace();
    let w = ws.path();
    // The thread is deleted before this process ever saw a child of it: the
    // delete still writes the cancelled-parent marker in the caller's workspace.
    discard_for_thread(w, "thread-race");
    // A straggler that records after the sweep (the cooperative-abort race) is
    // dropped rather than queued.
    record(w, "sess-race", "sub-late", "stale", Some("thread-race"));
    assert!(pending_ids(w, "thread-race").is_empty());
    // A different, live thread still records normally.
    record(w, "sess-race", "sub-ok", "fresh", Some("thread-live-race"));
    assert_eq!(pending_ids(w, "thread-live-race"), ["sub-ok"]);
}

#[test]
fn a_child_registering_after_its_thread_was_deleted_is_aborted_and_stays_dropped() {
    let _guard = test_guard();
    let ws = workspace();
    let w = ws.path();
    assert!(!mark_stopped_task_if_thread_stopped(
        w,
        "thread-del-reg",
        "sub-a"
    ));
    discard_for_thread(w, "thread-del-reg");

    // A child spawned before the delete registers after it: it must be aborted,
    // and registering must not lift the delete (unlike a stopped thread, a
    // deleted one never reopens).
    assert!(mark_stopped_task_if_thread_stopped(
        w,
        "thread-del-reg",
        "sub-b"
    ));
    record(w, "sess-del", "sub-a", "straggler", Some("thread-del-reg"));
    record(w, "sess-del", "sub-b", "late child", Some("thread-del-reg"));
    assert!(pending_ids(w, "thread-del-reg").is_empty());
}

#[test]
fn clear_all_withdraws_every_pending_completion() {
    let _guard = test_guard();
    let ws = workspace();
    let w = ws.path();
    record(w, "sess-c1", "sub-1", "a", Some("t1"));
    record(w, "sess-c2", "sub-2", "b", Some("t2"));

    let removed = clear_all(w);
    assert_eq!(removed, 2);
    assert!(pending_ids(w, "t1").is_empty());
    assert!(pending_ids(w, "t2").is_empty());
    assert_eq!(clear_all(w), 0);
}

#[test]
fn clear_all_is_scoped_to_its_workspace() {
    let _guard = test_guard();
    let purged = workspace();
    let other = workspace();
    record(purged.path(), "sess-p", "sub-1", "a", Some("t-purged"));
    record(other.path(), "sess-o", "sub-2", "b", Some("t-other"));

    assert_eq!(clear_all(purged.path()), 1);

    assert!(pending_ids(purged.path(), "t-purged").is_empty());
    assert_eq!(
        pending_ids(other.path(), "t-other"),
        ["sub-2"],
        "another workspace's completions are untouched"
    );
    record(other.path(), "sess-o", "sub-3", "c", Some("t-other"));
    assert_eq!(pending_ids(other.path(), "t-other").len(), 2);
}

#[test]
fn recovery_is_claimed_once_per_workspace() {
    let _guard = test_guard();
    let ws = workspace();
    assert!(claim_recovery(ws.path()));
    assert!(!claim_recovery(ws.path()));
    forget_workspace_for_test(ws.path());
    assert!(claim_recovery(ws.path()), "a restart scans again");
}

#[test]
fn mark_collected_sweeps_the_queued_entry() {
    let _guard = test_guard();
    let ws = workspace();
    let w = ws.path();
    record(w, "sess-mc", "mc-sub-1", "collected", Some("thread-mc"));
    record(w, "sess-mc", "mc-sub-2", "keep", Some("thread-mc"));

    // The parent collected sub-1 inline, so it must not be delivered again;
    // sub-2 (never waited on) survives for normal idle delivery.
    assert!(mark_collected(w, "mc-sub-1"), "swept the queued entry");
    assert_eq!(pending_ids(w, "thread-mc"), ["mc-sub-2"]);
}

#[test]
fn record_after_mark_collected_is_dropped() {
    let _guard = test_guard();
    let ws = workspace();
    let w = ws.path();
    assert!(
        !mark_collected(w, "mc-late"),
        "nothing queued yet, nothing swept"
    );
    // A completion that records *after* (the wait-before-record order) is
    // dropped rather than queued for a duplicate delivery turn.
    record(
        w,
        "sess-mc-race",
        "mc-late",
        "stale",
        Some("thread-mc-race"),
    );
    assert!(pending_ids(w, "thread-mc-race").is_empty());
}

#[test]
fn mark_collected_is_task_scoped() {
    let _guard = test_guard();
    let ws = workspace();
    let w = ws.path();
    // Only the collected task is suppressed; an un-waited sibling still
    // surfaces (the genuinely-later fire-and-forget feature is preserved).
    mark_collected(w, "mc-scope-1");
    record(
        w,
        "sess-mc-scope",
        "mc-scope-2",
        "later",
        Some("thread-mc-scope"),
    );
    assert_eq!(pending_ids(w, "thread-mc-scope"), ["mc-scope-2"]);
}

#[test]
fn record_failure_queues_a_framed_failure_for_delivery() {
    let _guard = test_guard();
    let ws = workspace();
    let w = ws.path();
    futures::executor::block_on(record_failure(
        w,
        "sess-fail",
        "sub-bad",
        "researcher",
        "provider exploded",
        Some("thread-fail".into()),
    ));
    let pending = pending_for(w, "thread-fail");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].status, CompletionStatus::Failed);
    assert!(pending[0].result.text.starts_with("[SUBAGENT_FAILED]"));
    assert!(pending[0].result.text.contains("provider exploded"));
}

#[test]
fn record_awaiting_input_queues_a_framed_needs_input_for_delivery() {
    let _guard = test_guard();
    let ws = workspace();
    let w = ws.path();
    futures::executor::block_on(record_awaiting_input(
        w,
        "sess-ask",
        "sub-ask",
        "researcher",
        "which repo?",
        true,
        Some("thread-ask".into()),
    ));
    let pending = pending_for(w, "thread-ask");
    assert_eq!(pending.len(), 1);
    assert_eq!(
        BackgroundAgentOutcome::of(&pending[0]),
        BackgroundAgentOutcome::AwaitingInput
    );
    assert!(pending[0].result.text.contains("which repo?"));
}

#[test]
fn the_outcome_survives_a_claim() {
    let _guard = test_guard();
    let ws = workspace();
    let w = ws.path();
    futures::executor::block_on(record_outcome(
        w,
        "sess-out",
        "sub-1",
        "researcher",
        "[SUBAGENT_FAILED] boom",
        Some("thread-out".into()),
        BackgroundAgentOutcome::Failed,
    ));
    let claimed = router_for_workspace(w)
        .claim_pending("thread-out", usize::MAX)
        .unwrap();
    assert_eq!(
        BackgroundAgentOutcome::of(&claimed[0]),
        BackgroundAgentOutcome::Failed
    );
}

#[test]
fn discard_pending_for_thread_blocks_late_results_until_the_next_turn() {
    let _guard = test_guard();
    let ws = workspace();
    let w = ws.path();
    record(
        w,
        "sess-stop",
        "sub-stop-a",
        "finished before Stop",
        Some("thread-stop-live"),
    );
    record(
        w,
        "sess-stop",
        "sub-stop-keep",
        "other thread",
        Some("thread-stop-other"),
    );

    // Stop drops the undelivered result so it can't start a delivery turn...
    assert_eq!(discard_pending_for_thread("thread-stop-live"), 1);
    assert!(pending_ids(w, "thread-stop-live").is_empty());
    assert_eq!(pending_ids(w, "thread-stop-other"), ["sub-stop-keep"]);

    // A late completion from the stopped generation loses the cooperative
    // abort race and is rejected.
    record(
        w,
        "sess-stop",
        "sub-stop-later",
        "late",
        Some("thread-stop-live"),
    );
    assert!(pending_ids(w, "thread-stop-live").is_empty());

    // Completing Stop keeps the thread gate until a new user turn begins, so a
    // child that registers after the cancellation sweep is still rejected.
    finish_stop_for_thread("thread-stop-live", &["sub-stop-later".into()]);
    record(
        w,
        "sess-stop",
        "sub-stop-later",
        "again",
        Some("thread-stop-live"),
    );
    assert!(pending_ids(w, "thread-stop-live").is_empty());

    // A child that was spawned before Stop but registers after the registry
    // sweep gets its own tombstone before the thread can be reopened.
    assert!(mark_stopped_task_if_thread_stopped(
        w,
        "thread-stop-live",
        "sub-stop-registered-late"
    ));

    resume_stopped_thread("thread-stop-live");
    record(
        w,
        "sess-stop",
        "sub-stop-registered-late",
        "late reg",
        Some("thread-stop-live"),
    );
    assert!(
        pending_ids(w, "thread-stop-live").is_empty(),
        "the stopped generation's task ids stay tombstoned after the thread reopens"
    );
    record(
        w,
        "sess-stop",
        "sub-stop-new-turn",
        "next turn",
        Some("thread-stop-live"),
    );
    assert_eq!(pending_ids(w, "thread-stop-live"), ["sub-stop-new-turn"]);
}

#[test]
fn a_stop_before_a_restart_is_lifted_by_the_first_new_spawn() {
    let _guard = test_guard();
    let ws = workspace();
    let w = ws.path();
    record(w, "sess-rs", "sub-1", "x", Some("thread-restart-stop"));
    assert_eq!(discard_pending_for_thread("thread-restart-stop"), 1);

    // A restart: the in-memory gate is gone, the durable cancelled-parent
    // marker is not.
    forget_workspace_for_test(w);
    assert!(!mark_stopped_task_if_thread_stopped(
        w,
        "thread-restart-stop",
        "sub-new"
    ));
    record(
        w,
        "sess-rs",
        "sub-new",
        "after restart",
        Some("thread-restart-stop"),
    );
    assert_eq!(
        pending_ids(w, "thread-restart-stop"),
        ["sub-new"],
        "a new spawn proves the user re-engaged the thread"
    );
}

#[test]
fn undelivered_completions_are_found_again_after_a_restart() {
    let _guard = test_guard();
    let ws = workspace();
    let w = ws.path();
    record(
        w,
        "sess-boot",
        "sub-1",
        "unseen result",
        Some("thread-boot"),
    );
    record(
        w,
        "sess-boot",
        "sub-2",
        "delivered result",
        Some("thread-boot-done"),
    );
    let router = router_for_workspace(w);
    router
        .claim_pending("thread-boot-done", usize::MAX)
        .unwrap();
    router.mark_delivered(&["sub-2"]).unwrap();
    drop(router);

    forget_workspace_for_test(w);
    let threads = recover_pending_threads(w);

    assert_eq!(
        threads,
        ["thread-boot"],
        "only the undelivered thread is recovered"
    );
    assert_eq!(
        pending_for(w, "thread-boot")[0].result.text,
        "unseen result"
    );
    assert!(router_for_thread("thread-boot").is_some());
}

#[test]
fn a_delete_survives_a_restart_and_is_not_lifted_by_a_new_spawn() {
    let _guard = test_guard();
    let ws = workspace();
    let w = ws.path();
    record(w, "sess-dr", "sub-1", "x", Some("thread-del-restart"));
    assert_eq!(discard_for_thread(w, "thread-del-restart"), 1);

    // A restart: memory is gone, the durable markers are not. A late child for
    // the deleted thread must be aborted rather than treated as a user returning
    // to a stopped thread, and its result must stay dropped.
    forget_workspace_for_test(w);
    assert!(mark_stopped_task_if_thread_stopped(
        w,
        "thread-del-restart",
        "sub-late"
    ));
    record(
        w,
        "sess-dr",
        "sub-late",
        "stale",
        Some("thread-del-restart"),
    );
    assert!(pending_ids(w, "thread-del-restart").is_empty());
}

#[test]
fn in_memory_gates_drop_a_result_even_without_a_durable_marker() {
    let _guard = test_guard();
    let ws = workspace();
    let w = ws.path();
    // The gates are what protects the process if the durable cancel write fails.
    state().deleted_threads.insert("thread-gate-del".into());
    state().stopped_threads.insert("thread-gate-stop".into());
    record(w, "sess-g", "sub-1", "x", Some("thread-gate-del"));
    record(w, "sess-g", "sub-2", "y", Some("thread-gate-stop"));
    assert!(pending_ids(w, "thread-gate-del").is_empty());
    assert!(pending_ids(w, "thread-gate-stop").is_empty());
    state().deleted_threads.remove("thread-gate-del");
    state().stopped_threads.remove("thread-gate-stop");
}

#[test]
fn clear_all_also_withdraws_a_completion_a_delivery_has_leased() {
    let _guard = test_guard();
    let ws = workspace();
    let w = ws.path();
    record(w, "sess-l", "sub-1", "x", Some("thread-leased"));
    let router = router_for_workspace(w);
    // A delivery claimed it: the record is leased but still pending in the store.
    assert_eq!(
        router
            .claim_pending("thread-leased", usize::MAX)
            .unwrap()
            .len(),
        1
    );

    assert_eq!(clear_all(w), 1);

    assert!(pending_ids(w, "thread-leased").is_empty());
    assert_eq!(
        router.mark_delivered(&["sub-1"]).unwrap(),
        0,
        "nothing left to settle"
    );
}

#[test]
fn the_session_cache_evicts_its_oldest_mapping_only() {
    isolation::assert_session_cache_eviction();
}

#[path = "background_completions_isolation_tests.rs"]
mod isolation;

#[test]
fn a_delete_marker_outlives_compaction() {
    let _guard = test_guard();
    let ws = workspace();
    let w = ws.path();
    record(w, "sess-cm", "sub-1", "x", Some("thread-compact"));
    discard_for_thread(w, "thread-compact");

    // Settled records expire; the deletion must not.
    router_for_workspace(w)
        .compact(std::time::Duration::ZERO)
        .unwrap();
    forget_workspace_for_test(w);
    assert!(mark_stopped_task_if_thread_stopped(
        w,
        "thread-compact",
        "sub-late"
    ));
}

#[test]
fn deleting_a_thread_that_never_used_background_work_writes_nothing() {
    let _guard = test_guard();
    let ws = workspace();
    let w = ws.path();
    assert_eq!(discard_for_thread(w, "thread-plain"), 0);
    assert!(
        entry_for(w).store.list(None).is_empty(),
        "no marker is persisted for an ordinary thread"
    );
    // The in-memory gate still holds for this process.
    record(w, "sess-pl", "sub-1", "x", Some("thread-plain"));
    assert!(pending_ids(w, "thread-plain").is_empty());
}

#[test]
fn a_late_record_for_a_deleted_thread_is_dropped_by_the_durable_marker_alone() {
    let _guard = test_guard();
    let ws = workspace();
    let w = ws.path();
    record(w, "sess-dm", "sub-1", "x", Some("thread-dm"));
    discard_for_thread(w, "thread-dm");
    // A restart empties the in-memory gates; the marker on disk still decides.
    forget_workspace_for_test(w);
    record(w, "sess-dm", "sub-late", "stale", Some("thread-dm"));
    assert!(pending_ids(w, "thread-dm").is_empty());
}

#[tokio::test]
async fn release_all_closes_every_router_and_the_log_stays_replayable() {
    let _guard = crate::config::TEST_ENV_LOCK.lock().await;
    let ws = workspace();
    let w = ws.path();
    record_completion(
        w,
        "sess-rel",
        "sub-1",
        "researcher",
        "kept on disk",
        Some("thread-rel".into()),
    )
    .await;

    assert!(release_all().await >= 1);
    assert!(
        !state().routers.contains_key(w),
        "the router (and its log handle) is dropped"
    );
    assert!(
        workspace_for_thread("thread-rel").is_none(),
        "process-local maps are cleared"
    );

    // The next boot finds the undelivered completion from the log alone.
    assert_eq!(recover_pending_threads(w), ["thread-rel"]);
}

#[tokio::test(start_paused = true)]
async fn release_all_waits_for_a_router_still_in_use() {
    let _guard = crate::config::TEST_ENV_LOCK.lock().await;
    let ws = workspace();
    let w = ws.path();
    let in_use = router_for_workspace(w);
    let releasing = tokio::spawn(release_all());
    // Well inside the drain window: still waiting on the held router.
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    assert!(
        !releasing.is_finished(),
        "waits while a delivery holds the router"
    );
    drop(in_use);
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert!(releasing.await.unwrap() >= 1);
}

#[tokio::test(start_paused = true)]
async fn release_all_gives_up_waiting_after_the_drain_window() {
    let _guard = crate::config::TEST_ENV_LOCK.lock().await;
    let ws = workspace();
    let w = ws.path();
    let _held_forever = router_for_workspace(w);
    let released = release_all().await;
    assert!(
        released >= 1,
        "stragglers are dropped once the window passes"
    );
    assert!(!state().routers.contains_key(w));
}

#[tokio::test]
async fn release_all_keeps_the_stop_and_delete_gates() {
    let _guard = crate::config::TEST_ENV_LOCK.lock().await;
    let ws = workspace();
    let w = ws.path();
    discard_pending_for_thread("thread-keep-stop");
    state().deleted_threads.insert("thread-keep-del".into());
    release_all().await;

    // A detached child finishing after the release is still refused.
    record_completion(w, "s", "sub-1", "r", "x", Some("thread-keep-stop".into())).await;
    record_completion(w, "s", "sub-2", "r", "x", Some("thread-keep-del".into())).await;
    assert!(pending_ids(w, "thread-keep-stop").is_empty());
    assert!(pending_ids(w, "thread-keep-del").is_empty());
    resume_stopped_thread("thread-keep-stop");
    state().deleted_threads.remove("thread-keep-del");
}

#[test]
fn a_completion_for_a_removed_workspace_does_not_recreate_it() {
    let _guard = test_guard();
    let ws = workspace();
    let gone = ws.path().join("reset-away");
    record(&gone, "sess-gone", "sub-1", "x", Some("thread-gone"));
    assert!(!gone.exists(), "the workspace directory is not recreated");
}

/// A store whose writes always fail, like a full disk.
struct FullDisk(InMemoryCompletionStore);

impl CompletionStore for FullDisk {
    fn get(&self, task_id: &str) -> Option<CompletionRecord> {
        self.0.get(task_id)
    }
    fn put(&self, _record: &CompletionRecord) -> tinyagents_harness::error::Result<()> {
        Err(tinyagents_harness::error::TinyAgentsError::Graph(
            "disk full".into(),
        ))
    }
    fn list(&self, parent_key: Option<&str>) -> Vec<CompletionRecord> {
        self.0.list(parent_key)
    }
}

#[tokio::test]
async fn a_store_that_keeps_failing_degrades_to_memory_instead_of_losing_the_result() {
    let _guard = crate::config::TEST_ENV_LOCK.lock().await;
    let ws = workspace();
    let w = ws.path();
    install_store_for_test(w, Arc::new(FullDisk(InMemoryCompletionStore::new())));

    record_completion(
        w,
        "sess-fd",
        "sub-1",
        "researcher",
        "must not be lost",
        Some("thread-fd".into()),
    )
    .await;

    let pending = pending_for(w, "thread-fd");
    assert_eq!(pending.len(), 1, "the result is held in memory");
    assert_eq!(pending[0].result.text, "must not be lost");
}

#[tokio::test]
async fn concurrent_degradation_installs_one_fallback() {
    let _guard = crate::config::TEST_ENV_LOCK.lock().await;
    let ws = workspace();
    let w = ws.path();
    let failing = router_for_workspace(w);

    // Two records saw the same failing router; both ask to degrade.
    let first = degrade_to_memory(w, &failing);
    let second = degrade_to_memory(w, &failing);

    assert!(
        Arc::ptr_eq(&first.router, &second.router),
        "the second caller reuses the fallback the first installed"
    );
    assert!(Arc::ptr_eq(&entry_for(w).router, &first.router));
}
