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
    let path = run_log_path(dir.path(), "workflow", "run-stop-123");
    write_header(
        dir.path(),
        &path,
        "workflow",
        "run-stop-123",
        &serde_json::json!({}),
        "prompt",
    )
    .await
    .unwrap();
    let (tx, rx) = tokio::sync::mpsc::channel(4);
    tx.send(AgentProgress::TurnCompleted {
        iterations: 5,
        stop: Some(tinyagents_harness::turn_stop::TurnStop::iteration_cap()),
    })
    .await
    .unwrap();
    drop(tx);
    let stop = drain_to_log(rx, dir.path().to_path_buf(), path.clone()).await;
    assert_eq!(
        stop,
        Some(tinyagents_harness::turn_stop::TurnStop::iteration_cap())
    );
    let log = read_run_log_slice(dir.path(), &path, 0, 4096).unwrap();
    assert!(log.content.contains("stopped: iteration_cap"), "{log:?}");
    write_footer(dir.path(), &path, "STOPPED", 1, "stopped")
        .await
        .unwrap();
}

#[tokio::test]
async fn host_written_log_parses_like_legacy_layout() {
    // Regression: the layout the host writes through the tinyskills module
    // must parse identically to logs written before the move.
    let tmp = tempfile::TempDir::new().unwrap();
    let path = run_log_path(tmp.path(), "triage-inbox", "run-abcdef123456");
    assert!(path.to_string_lossy().starts_with("skills.runs."));
    write_header(
        tmp.path(),
        &path,
        "triage-inbox",
        "run-abcdef123456",
        &serde_json::json!({}),
        "do it",
    )
    .await
    .unwrap();
    write_footer(tmp.path(), &path, "DONE", 42, "ok")
        .await
        .unwrap();
    let runs = scan_runs(tmp.path(), Some("triage-inbox"), 10);
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].run_id, "run-abcdef123456");
    assert_eq!(
        find_run_log_path(tmp.path(), "run-abcdef123456"),
        Some(path.clone())
    );
    assert_eq!(runs[0].status, "DONE");
    assert_eq!(runs[0].duration_ms, Some(42));
    assert_eq!(
        read_terminal_outcome(tmp.path(), &path).unwrap().status,
        "DONE"
    );

    // The stream retains the same parsed run metadata as the old log format.
}

#[test]
fn a_compaction_is_logged_as_a_step() {
    let line = format_event(&AgentProgress::ContextCompacting).expect("a step line");
    assert!(
        line.ends_with("· summarizing earlier conversation"),
        "{line}"
    );
}

/// The run's progress bridge is spawned from the detached run task. The log
/// store resolves its storage scope from the acting context, so the bridge
/// must act for the dispatching agent: spawned bare, its steps land under the
/// default scope while the header sits under the agent's, and the agent's log
/// never shows them.
#[tokio::test]
async fn the_progress_bridge_writes_under_the_dispatching_agents_scope() {
    use crate::core::runtime::{ContextOverlay, CoreContext, DispatchCtx, DomainSet};

    let ctx = CoreContext::for_test(DomainSet::full(), None).derive_with(
        ContextOverlay::new(
            crate::config::Config::default(),
            DomainSet::full(),
            Default::default(),
        )
        .session_agent("u-asha"),
    );
    let dir = tempfile::tempdir().expect("tempdir");
    let path = run_log_path(dir.path(), "workflow", "run-scope-123");
    CoreContext::scope(
        ctx.clone(),
        write_header(
            dir.path(),
            &path,
            "workflow",
            "run-scope-123",
            &serde_json::json!({}),
            "prompt",
        ),
    )
    .await
    .unwrap();
    let (tx, rx) = tokio::sync::mpsc::channel(4);
    tx.send(AgentProgress::TurnCompleted {
        iterations: 5,
        stop: Some(tinyagents_harness::turn_stop::TurnStop::iteration_cap()),
    })
    .await
    .unwrap();
    drop(tx);

    // Spawned from outside any scope, as the run task's bridge is.
    spawn_drain_to_log(
        &DispatchCtx::new(ctx.clone()),
        rx,
        dir.path().to_path_buf(),
        path.clone(),
    )
    .await
    .unwrap();

    let log = CoreContext::scope(ctx, async {
        read_run_log_slice(dir.path(), &path, 0, 4096)
    })
    .await
    .unwrap();
    assert!(log.content.contains("stopped: iteration_cap"), "{log:?}");
}
