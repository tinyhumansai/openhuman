use super::*;

#[tokio::test]
async fn dedup_commit_two_dedup_nodes_settle_independently() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow = Flow {
        id: "f-multi".to_string(),
        name: "f-multi".to_string(),
        enabled: true,
        graph: WorkflowGraph {
            nodes: vec![
                trigger_node(json!({})),
                dedup_node("dd1"),
                dedup_node("dd2"),
            ],
            ..Default::default()
        },
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
        last_run_at: None,
        last_status: None,
        require_approval: false,
    };
    store::upsert_flow(&config, &flow).unwrap();

    let namespace = dedup_state_namespace("f-multi");
    store::kv_set(&config, &namespace, "dedup:dd1:tentative", &json!(["a"])).unwrap();
    store::kv_set(&config, &namespace, "dedup:dd2:tentative", &json!(["b"])).unwrap();

    let sub = DedupCommitSubscriber::new(config.clone());
    sub.handle(&DomainEvent::FlowRunFinished {
        flow_id: "f-multi".into(),
        run_id: "run-multi".into(),
        status: "completed".into(),
    })
    .await;

    assert_eq!(
        store::kv_get(&config, &namespace, "dedup:dd1:committed")
            .unwrap()
            .unwrap(),
        json!(["a"])
    );
    assert_eq!(
        store::kv_get(&config, &namespace, "dedup:dd2:committed")
            .unwrap()
            .unwrap(),
        json!(["b"])
    );
}

// ── per-flow commit serialization (issue #5265) ───────────────────
//
// CodeRabbit "Major" on the dedup engine PR: the commit's
// load(committed)+union(tentative)+store(committed) is a
// read-modify-write, not a CAS. Two overlapping `FlowRunFinished`
// events for the SAME flow could otherwise interleave and have the
// second writer's store clobber the first writer's union, silently
// losing that run's committed keys. `handle_finished` now serializes
// settlement per `flow_id` via `FLOW_COMMIT_LOCKS`.
//
// Two tests, deliberately split:
//
// - `..._never_runs_two_commits_for_the_same_flow_concurrently` spawns a
//   burst of genuinely overlapping `FlowRunFinished` events for the SAME
//   flow_id and proves the LOCK itself provides mutual exclusion (the
//   high-water mark of concurrently-active critical sections never
//   exceeds 1) — this is the "spawn two tasks contending on the same
//   flow_id" case.
// - `..._serial_commits_for_the_same_flow_accumulate_via_union` proves
//   the property that mutual exclusion protects: settling run after run
//   for the same node never clobbers an earlier run's committed keys —
//   each contributes to the union.
//
// These are split rather than combined into one "two runs with two
// different tentative sets, truly concurrently, assert union" test
// because `tentative` is a single shared KV row per node (not
// per-run) — forcing two *different* tentative contents to both survive
// a genuinely simultaneous read would require injecting a write from
// outside `handle_finished` in the middle of its critical section, which
// instead exercises the SEPARATE, still-open node-side race (the
// `dedup` node's own in-run `tentative` read-modify-write, documented on
// `DedupCommitSubscriber` above as explicitly NOT fixed by this lock).
// Together, the two tests below establish the same guarantee end to
// end: the lock enforces serialization (test 1), and serialization is
// sufficient for correctness (test 2).

#[tokio::test]
async fn dedup_commit_never_runs_two_commits_for_the_same_flow_concurrently() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow = flow_with_dedup_node("f-race", "dd");
    store::upsert_flow(&config, &flow).unwrap();

    let namespace = dedup_state_namespace("f-race");
    store::kv_set(&config, &namespace, "dedup:dd:tentative", &json!(["seed"])).unwrap();

    // Arm the test-only scheduling hook (see `CommitTestHooks`): every
    // `handle_finished` call sleeps briefly while holding the per-flow
    // lock, and records how many calls are concurrently inside that
    // window. Instance-scoped (not a global static) so this doesn't
    // interfere with — or get polluted by — unrelated tests that cargo
    // runs concurrently on other threads. Without a correctly-scoped
    // lock, a burst of overlapping `FlowRunFinished` events for the SAME
    // flow_id would pile up inside the critical section together
    // instead of queuing.
    let hooks = Arc::new(CommitTestHooks::default());
    hooks
        .delay_ms
        .store(20, std::sync::atomic::Ordering::SeqCst);

    let sub = Arc::new(DedupCommitSubscriber::with_test_hooks(
        config.clone(),
        hooks.clone(),
    ));
    let mut handles = Vec::new();
    for i in 0..5 {
        let sub = sub.clone();
        handles.push(tokio::spawn(async move {
            sub.handle(&DomainEvent::FlowRunFinished {
                flow_id: "f-race".into(),
                run_id: format!("run-{i}"),
                status: "completed".into(),
            })
            .await;
        }));
    }
    for handle in handles {
        handle.await.unwrap();
    }

    assert_eq!(
        hooks.concurrent.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "every critical-section entry must have a matching exit"
    );
    assert_eq!(
        hooks
            .max_concurrent
            .load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the per-flow lock must serialize overlapping FlowRunFinished handling for the \
         same flow_id — at most one commit critical section may be active at a time"
    );
}

#[tokio::test]
async fn dedup_commit_serial_commits_for_the_same_flow_accumulate_via_union() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow = flow_with_dedup_node("f-serial", "dd");
    store::upsert_flow(&config, &flow).unwrap();

    let namespace = dedup_state_namespace("f-serial");
    let sub = DedupCommitSubscriber::new(config.clone());

    // Run A finishes, having tentatively seen "a".
    store::kv_set(&config, &namespace, "dedup:dd:tentative", &json!(["a"])).unwrap();
    sub.handle(&DomainEvent::FlowRunFinished {
        flow_id: "f-serial".into(),
        run_id: "run-a".into(),
        status: "completed".into(),
    })
    .await;

    // Run B finishes later, having independently tentatively seen "b".
    // The per-flow lock (proven by the concurrency test above) is what
    // guarantees two overlapping runs' `FlowRunFinished` handling
    // reduces to exactly this serialized order in practice — so this is
    // the correctness property that mutual exclusion is protecting.
    store::kv_set(&config, &namespace, "dedup:dd:tentative", &json!(["b"])).unwrap();
    sub.handle(&DomainEvent::FlowRunFinished {
        flow_id: "f-serial".into(),
        run_id: "run-b".into(),
        status: "completed".into(),
    })
    .await;

    let committed = store::kv_get(&config, &namespace, "dedup:dd:committed")
        .unwrap()
        .expect("committed key must exist after both runs settle");
    let mut committed: Vec<&str> = committed
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    committed.sort_unstable();
    assert_eq!(
        committed,
        vec!["a", "b"],
        "settling run B must not clobber run A's already-committed keys — committed is a \
         running union across every run that has settled, never a last-writer-wins overwrite"
    );
    assert!(
        store::kv_get(&config, &namespace, "dedup:dd:tentative")
            .unwrap()
            .is_none(),
        "tentative must be cleared after each successful commit"
    );
}

#[test]
fn flow_commit_lock_returns_the_same_arc_for_the_same_flow_id_and_differs_across_flows() {
    let a1 = flow_commit_lock("f-lock-a");
    let a2 = flow_commit_lock("f-lock-a");
    assert!(
        Arc::ptr_eq(&a1, &a2),
        "the same flow_id must share one lock instance"
    );

    let b = flow_commit_lock("f-lock-b");
    assert!(
        !Arc::ptr_eq(&a1, &b),
        "different flow_ids must not contend on the same lock"
    );
}
