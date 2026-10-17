//! Literal wire fixtures for the running-sub-agent surface: status labels the
//! roster/UI read, the durable ledger's JSONL record shape, the durable→status
//! mapping `wait` falls back to, and the cancel `outcome` strings. These pin
//! the bytes; they must not change when the generic logic moves upstream.

use super::*;
use crate::agent::orchestration::running_subagents::roster::snapshot_for_parent;
use crate::agent::orchestration::running_subagents::task_ledger::task_store_for_workspace;
use tinyagents_harness::ids::TaskId;
use tinyagents_orchestration::subagent::FinishedOutcome;
use tinyagents_orchestration::subagent::WaitError;
use tinyagents_tasks::{OrchestrationTaskKind, OrchestrationTaskResult, OrchestrationTaskSpec};

fn spec(id: &str, parent: &str, session: Option<&str>) -> OrchestrationTaskSpec {
    let mut spec = OrchestrationTaskSpec::new(
        id.to_string(),
        OrchestrationTaskKind::SubAgent {
            agent: "researcher".to_string(),
        },
    )
    .with_lineage(parent.to_string(), parent.to_string())
    .with_metadata("parentSession", parent.to_string());
    if let Some(session) = session {
        spec = spec.with_metadata("subagentSessionId", session.to_string());
    }
    spec
}

#[tokio::test]
async fn roster_status_labels_are_literal() {
    let _guard = super::test_guard_async().await;
    let mut senders = Vec::new();
    for id in ["wire-run", "wire-done", "wire-await", "wire-fail"] {
        let (tx, rx) = status_channel();
        register(
            &crate::core::runtime::DispatchCtx::ambient(),
            id.into(),
            "researcher".into(),
            "wire-parent".into(),
            None,
            None,
            super::test_workspace(),
            None,
            super::run_queue(),
            super::dummy_abort(),
            rx,
        )
        .expect("fixture registration admitted");
        senders.push((id, tx));
    }
    for (id, tx) in &senders {
        match *id {
            "wire-done" => tx
                .send(DetachedSubagentStatus::Completed {
                    output: "o".into(),
                    iterations: 1,
                })
                .unwrap(),
            "wire-await" => tx
                .send(DetachedSubagentStatus::AwaitingUser {
                    question: "q".into(),
                })
                .unwrap(),
            "wire-fail" => tx
                .send(DetachedSubagentStatus::Failed { error: "e".into() })
                .unwrap(),
            _ => {}
        }
    }
    let labels: Vec<(String, &str)> =
        snapshot_for_parent(&crate::core::runtime::DispatchCtx::ambient(), "wire-parent")
            .into_iter()
            .map(|s| (s.task_id, s.status))
            .collect();
    assert_eq!(
        labels,
        vec![
            ("wire-await".to_string(), "awaiting_user"),
            ("wire-done".to_string(), "completed"),
            ("wire-fail".to_string(), "failed"),
            ("wire-run".to_string(), "running"),
        ]
    );
    for (id, _) in senders {
        prune(id);
    }
}

#[test]
fn cancel_outcome_strings_are_literal() {
    assert_eq!(FinishedOutcome::Completed.as_str(), "completed");
    assert_eq!(FinishedOutcome::Failed.as_str(), "failed");
}

#[tokio::test]
async fn durable_fallback_maps_every_store_status_to_literal_wait_outcome() {
    let ws = super::test_workspace();
    let store = task_store_for_workspace(&ws);
    let parent = "wire-durable-parent";
    let timeout = Duration::from_millis(5);
    let mut seen = Vec::new();

    let mk = |id: &str| {
        store.insert(spec(id, parent, None)).unwrap();
        store.mark_running(&TaskId::new(id)).unwrap();
        TaskId::new(id)
    };
    let id = mk("wire-d-completed");
    store
        .complete(&id, OrchestrationTaskResult::text("final text".to_string()))
        .unwrap();
    let id = mk("wire-d-failed");
    store.fail(&id, "boom".to_string()).unwrap();
    let id = mk("wire-d-awaiting");
    store.mark_awaiting(&id).unwrap();
    let id = mk("wire-d-cancelled");
    store.request_cancel(&id).unwrap();
    store.mark_cancelled(&id).unwrap();
    mk("wire-d-running");
    // Awaiting with no recorded question / failure with no recorded error use
    // the fallback sentences.
    store.insert(spec("wire-d-pending", parent, None)).unwrap();

    for id in [
        "wire-d-completed",
        "wire-d-failed",
        "wire-d-awaiting",
        "wire-d-cancelled",
        "wire-d-running",
        "wire-d-pending",
    ] {
        let outcome = wait_in_workspace(
            &crate::core::runtime::DispatchCtx::ambient(),
            id,
            parent,
            &ws,
            timeout,
        )
        .await;
        seen.push(format!("{id}: {outcome:?}"));
    }
    assert_eq!(
        seen,
        vec![
            "wire-d-completed: Ok(Terminal(Completed { output: \"final text\", iterations: 0 }))",
            "wire-d-failed: Ok(Terminal(Failed { error: \"boom\" }))",
            "wire-d-awaiting: Ok(Terminal(AwaitingUser { question: \"sub-agent is awaiting user input; no clarification text was available from the durable task store\" }))",
            "wire-d-cancelled: Ok(Terminal(Failed { error: \"sub-agent was cancelled\" }))",
            "wire-d-running: Ok(TimedOut(Running))",
            "wire-d-pending: Ok(TimedOut(Running))",
        ]
    );
    // A different parent is refused, an unknown id is unknown.
    assert_eq!(
        wait_in_workspace(
            &crate::core::runtime::DispatchCtx::ambient(),
            "wire-d-failed",
            "someone-else",
            &ws,
            timeout
        )
        .await
        .unwrap_err(),
        WaitError::NotOwned
    );
    assert_eq!(
        wait_in_workspace(
            &crate::core::runtime::DispatchCtx::ambient(),
            "wire-d-nope",
            parent,
            &ws,
            timeout
        )
        .await
        .unwrap_err(),
        WaitError::Unknown
    );
}

#[tokio::test]
async fn spawn_ledger_record_has_literal_persisted_shape() {
    let _guard = super::test_guard_async().await;
    let ws = tempfile::tempdir().unwrap();
    let (_tx, rx) = status_channel();
    register(
        &crate::core::runtime::DispatchCtx::ambient(),
        "wire-shape-1".into(),
        "researcher".into(),
        "wire-shape-parent".into(),
        Some("rootrun__child".into()),
        Some("subsess-9".into()),
        ws.path().to_path_buf(),
        Some("thread-7".into()),
        super::run_queue(),
        super::dummy_abort(),
        rx,
    )
    .expect("fixture registration admitted");
    let record = super::task_ledger::list_task_records(ws.path())
        .into_iter()
        .next()
        .expect("a ledger record in the storage backend");
    let rendered = serde_json::to_value(&record).unwrap().to_string();
    for needle in [
        r#""parentSession":"wire-shape-parent""#,
        r#""rootSession":"rootrun""#,
        r#""defaultWaitTimeoutMs":"120000""#,
        r#""sessionParentPrefix":"rootrun__child""#,
        r#""parentThreadId":"thread-7""#,
        r#""subagentSessionId":"subsess-9""#,
        r#""workspaceDir":""#,
    ] {
        assert!(rendered.contains(needle), "{needle} missing in {rendered}");
    }
    assert!(rendered.contains("sub_agent") || rendered.contains("SubAgent"));
    println!("LEDGER_LINE {rendered}");
    prune("wire-shape-1");
}

#[test]
fn reconcile_reason_and_resume_labels_are_literal() {
    // Asserted through the durable record the reconciler writes.
    let ws = tempfile::tempdir().unwrap();
    let store = task_store_for_workspace(ws.path());
    store
        .insert(spec("wire-orphan-run", "wire-orphan-parent", None))
        .unwrap();
    store.mark_running(&TaskId::new("wire-orphan-run")).unwrap();
    store
        .insert(spec("wire-orphan-cancel", "wire-orphan-parent", None))
        .unwrap();
    store
        .mark_running(&TaskId::new("wire-orphan-cancel"))
        .unwrap();
    store
        .request_cancel(&TaskId::new("wire-orphan-cancel"))
        .unwrap();
    assert_eq!(reconcile_orphaned_tasks_on_boot(ws.path()), 2);
    let failed = store.get(&TaskId::new("wire-orphan-run")).unwrap();
    assert_eq!(
        failed.error.as_deref(),
        Some("sub-agent orphaned by core restart (was `running`)")
    );
    let cancelled = store.get(&TaskId::new("wire-orphan-cancel")).unwrap();
    assert_eq!(
        format!("{:?}", cancelled.status),
        "Cancelled",
        "cancel-requested orphans settle as cancelled"
    );
}
