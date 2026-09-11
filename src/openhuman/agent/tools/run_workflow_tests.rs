use super::*;

#[test]
fn run_workflow_name_and_schema_basics() {
    let t = RunWorkflowTool::new();
    assert_eq!(t.name(), "run_workflow");
    let schema = t.parameters_schema();
    let required = schema
        .get("required")
        .and_then(|v| v.as_array())
        .expect("required array");
    assert!(required.iter().any(|v| v.as_str() == Some("workflow_id")));
    // inputs is optional now (a workflow may declare no inputs).
    assert!(!required.iter().any(|v| v.as_str() == Some("inputs")));
}

#[test]
fn await_workflow_name_and_schema_basics() {
    let t = AwaitWorkflowTool::new();
    assert_eq!(t.name(), "await_workflow");
    let required = t
        .parameters_schema()
        .get("required")
        .and_then(|v| v.as_array())
        .cloned()
        .expect("required array");
    assert!(required.iter().any(|v| v.as_str() == Some("run_id")));
}

#[tokio::test]
async fn run_workflow_missing_id_returns_tool_error_not_panic() {
    let t = RunWorkflowTool::new();
    let res = t
        .execute(json!({"inputs": {}}))
        .await
        .expect("Ok(ToolResult)");
    assert!(res.is_error, "expected ToolResult::error");
    assert!(res.output().contains("workflow_id"));
}

#[tokio::test]
async fn await_workflow_missing_run_id_returns_tool_error() {
    let t = AwaitWorkflowTool::new();
    let res = t.execute(json!({})).await.expect("Ok(ToolResult)");
    assert!(res.is_error);
    assert!(res.output().contains("run_id"));
}

#[test]
fn detached_run_visibility_requires_profile_and_allowlist() {
    let run = crate::openhuman::skills::run_log::ScannedRun {
        run_id: "run-1".to_string(),
        workflow_id: "private-flow".to_string(),
        profile_id: Some("alice".to_string()),
        started: String::new(),
        status: "DONE".to_string(),
        duration_ms: None,
        finished: None,
        log_path: "/tmp/run.log".to_string(),
    };
    let allowed = ["private-flow".to_string()].into_iter().collect();
    let empty = std::collections::HashSet::new();

    assert!(run_visible_to_profile(
        &run,
        Some("alice"),
        Some(&allowed),
        &empty
    ));
    assert!(!run_visible_to_profile(
        &run,
        Some("bob"),
        Some(&allowed),
        &empty
    ));
    assert!(!run_visible_to_profile(
        &run,
        Some("alice"),
        Some(&empty),
        &empty
    ));
}

#[test]
fn wait_seconds_defaults_and_clamps() {
    assert_eq!(parse_wait_seconds(&json!({})), DEFAULT_WAIT_SECONDS);
    assert_eq!(parse_wait_seconds(&json!({"wait_seconds": 5})), 5);
    assert_eq!(parse_wait_seconds(&json!({"wait_seconds": 0})), 0);
    assert_eq!(
        parse_wait_seconds(&json!({"wait_seconds": 99_999})),
        MAX_WAIT_SECONDS
    );
}

#[test]
fn reentrancy_key_distinguishes_inputs() {
    let a = reentrancy_key("wf", &Some(json!({"pr": 1})));
    let b = reentrancy_key("wf", &Some(json!({"pr": 2})));
    let c = reentrancy_key("wf", &None);
    assert_ne!(a, b);
    assert_ne!(a, c);
    // Same id + same inputs → same key (so a tight loop is caught).
    assert_eq!(a, reentrancy_key("wf", &Some(json!({"pr": 1}))));
}

// ── Spawn-guard tests ────────────────────────────────────────────────
//
// The guards are process-global statics, so the await-slot tests share a
// lock to avoid clobbering each other's `ACTIVE_AWAITS`/`ACTIVE_KEYS` count
// under cargo's parallel runner. The RAII `AwaitGuard` frees its slot + key
// on drop, so these leave no residue (unlike the spawn backstop, which is
// monotonic by design — see its test).
fn guard_serial() -> &'static std::sync::Mutex<()> {
    static L: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    L.get_or_init(|| std::sync::Mutex::new(()))
}

#[test]
fn acquire_await_rejects_the_same_key_reentrantly() {
    let _s = guard_serial().lock().unwrap();
    let key = "reentry-test\u{1}null".to_string();
    let held = super::guard::acquire_await(key.clone()).expect("first acquire");
    let again = super::guard::acquire_await(key.clone());
    assert!(again.is_err(), "the same key while held must be rejected");
    assert!(again.err().unwrap().contains("re-entrant"));
    drop(held); // frees the key
    super::guard::acquire_await(key).expect("after drop the key is free again");
}

#[test]
fn acquire_await_caps_concurrent_awaits() {
    let _s = guard_serial().lock().unwrap();
    // MAX_ACTIVE_AWAITS is 8; hold 8 distinct keys, then the 9th must reject.
    let mut held = Vec::new();
    for i in 0..8 {
        held.push(super::guard::acquire_await(format!("cap-test-{i}")).expect("under the cap"));
    }
    let ninth = super::guard::acquire_await("cap-test-9".to_string());
    assert!(ninth.is_err(), "the 9th concurrent await must reject");
    assert!(ninth.err().unwrap().contains("already being awaited"));
    // Free one slot → the next acquire succeeds.
    held.pop();
    super::guard::acquire_await("cap-test-9".to_string()).expect("a freed slot is reusable");
}

#[tokio::test]
async fn unknown_workflow_id_does_not_burn_a_spawn_slot() {
    // Regression: a rejected spawn (unknown workflow id) must NOT consume a
    // slot against the process-lifetime backstop. `account_spawn` runs only
    // in the `Ok(started)` arm — after `spawn_workflow_run_background`
    // succeeds — so an unknown id (which fails synchronously) never accounts
    // a spawn. Without this ordering, an agent retrying a bad id would
    // exhaust the 500-spawn budget for legitimate runs. Asserts the counter
    // DELTA is zero (the counter is global + monotonic, so absolute value is
    // shared with the backstop test — hence the serial lock + delta check).
    let _s = guard_serial().lock().unwrap();
    let before = super::guard::total_spawns();
    let t = RunWorkflowTool::new();
    // wait_seconds: 0 → fire-and-forget path (no await slot taken); the
    // unknown id makes the spawn fail before accounting.
    let res = t
        .execute(json!({
            "workflow_id": "definitely-not-a-real-workflow-zzz",
            "wait_seconds": 0
        }))
        .await
        .expect("Ok(ToolResult)");
    assert!(res.is_error, "unknown workflow id must return a tool error");
    assert!(
        res.output().contains("unknown") || res.output().contains("workflow"),
        "error should reference the unknown workflow: {}",
        res.output()
    );
    let after = super::guard::total_spawns();
    assert_eq!(
        before, after,
        "a rejected spawn must not increment the spawn backstop counter"
    );
}

#[test]
fn account_spawn_trips_the_process_backstop() {
    let _s = guard_serial().lock().unwrap();
    // TOTAL_SPAWN_BACKSTOP is 500 and the counter is process-global +
    // monotonic (no reset by design — it's a runaway-loop backstop). Drive
    // well past it and assert it trips. NOTE: this permanently trips the
    // backstop for the rest of the process, which is fine because no other
    // non-ignored test calls `account_spawn` (only the #[ignore] e2e run
    // path does, and that runs in a separate process).
    let mut last = Ok(());
    for _ in 0..600 {
        last = super::guard::account_spawn();
        if last.is_err() {
            break;
        }
    }
    let err = last.expect_err("the spawn backstop must trip within 600 accounted spawns");
    assert!(err.contains("backstop"), "got: {err}");
}
