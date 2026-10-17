use super::*;

#[test]
fn run_cancel_registry_roundtrip() {
    let token = register_run_cancel("run-roundtrip");
    assert!(!token.is_cancelled());
    // Cancelling a registered run fires its token and reports found.
    assert!(cancel_run("run-roundtrip"));
    assert!(token.is_cancelled());
    // Unknown id ⇒ not found.
    assert!(!cancel_run("run-does-not-exist"));
    // After unregister the run is no longer cancellable.
    unregister_run_cancel("run-roundtrip");
    assert!(!cancel_run("run-roundtrip"));
}

#[test]
fn noisy_events_are_skipped_steps_are_kept() {
    assert!(format_event(&AgentProgress::TextDelta {
        delta: "hi".into(),
        iteration: 1
    })
    .is_none());
    // Content (prompt/reply) rides its own event and is never logged here.
    assert!(format_event(&AgentProgress::TurnContent {
        input: Some("secret prompt".into()),
        output: Some("secret reply".into()),
    })
    .is_none());
    let line = format_event(&AgentProgress::ToolCallStarted {
        call_id: "c1".into(),
        tool_name: "memory_search".into(),
        arguments: serde_json::json!({"query": "x"}),
        iteration: 2,
        display_label: None,
        display_detail: None,
    })
    .expect("tool call logged");
    assert!(line.contains("memory_search"));
    assert!(line.contains("it 2"));
}

/// A workflow run whose turn the harness stopped early (breaker, wind-down,
/// iteration cap) must not be written up as `DONE`: the drain hands the stop
/// back so the footer can say `STOPPED`, and the step log names it.
#[tokio::test]
async fn drain_to_log_returns_the_turns_stop() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("run.log");
    let (tx, rx) = tokio::sync::mpsc::channel(4);
    tx.send(AgentProgress::TurnCompleted {
        iterations: 5,
        stop: Some(tinyagents_harness::turn_stop::TurnStop::iteration_cap()),
    })
    .await
    .unwrap();
    drop(tx);
    let stop = drain_to_log(rx, path.clone()).await;
    assert_eq!(
        stop,
        Some(tinyagents_harness::turn_stop::TurnStop::iteration_cap())
    );
    let log = std::fs::read_to_string(&path).expect("log written");
    assert!(log.contains("stopped: iteration_cap"), "{log}");
}

#[tokio::test]
async fn host_written_log_parses_like_legacy_layout() {
    // Regression: the layout the host writes through the tinyskills module
    // must parse identically to logs written before the move.
    let tmp = tempfile::TempDir::new().unwrap();
    let path = run_log_path(tmp.path(), "triage-inbox", "run-abcdef123456");
    assert!(path.starts_with(runs_dir(tmp.path())));
    write_header(
        &path,
        "triage-inbox",
        "run-abcdef123456",
        &serde_json::json!({}),
        "do it",
    )
    .await
    .unwrap();
    write_footer(&path, "DONE", 42, "ok").await.unwrap();
    let runs = scan_runs(tmp.path(), Some("triage-inbox"), 10);
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].run_id, "run-abcdef123456");
    assert_eq!(runs[0].status, "DONE");
    assert_eq!(runs[0].duration_ms, Some(42));
    assert_eq!(read_terminal_outcome(&path).unwrap().status, "DONE");

    // A pre-existing log in the legacy format still parses.
    let legacy = runs_dir(tmp.path()).join("old_20260528T075113Z_aaaaaaaa.log");
    std::fs::write(
        &legacy,
        "==== workflow_run: old ====\nrun_id : aaaaaaaa-1\nstarted: 2026-05-28T07:51:13+00:00 UTC\ninputs : {}\n\n--- task prompt ---\nx\n\n--- steps ---\n\n--- result ---\nstatus  : FAILED\nduration: 5 ms\nfinished: 2026-05-28T07:51:14+00:00 UTC\n\nboom\n",
    )
    .unwrap();
    let old = scan_runs(tmp.path(), Some("old"), 10);
    assert_eq!(old[0].status, "FAILED");
    assert_eq!(old[0].duration_ms, Some(5));
}
