use super::*;

fn failed(event: AgentProgress) -> (String, String, String) {
    match event {
        AgentProgress::SubagentFailed {
            agent_id,
            task_id,
            error,
        } => (agent_id, task_id, error),
        other => panic!("expected SubagentFailed, got {other:?}"),
    }
}

/// The bug: an aborted background task is dropped mid-`.await`, so it never
/// reports. Aborting a real spawned task that holds an armed guard must still
/// deliver `SubagentFailed` on the parent's channel.
#[tokio::test]
async fn an_aborted_task_still_reports_failed() {
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let _report = AbortReport::arm(Some(tx), "agent_memory", "sub-1");
        let _ = started_tx.send(());
        std::future::pending::<()>().await;
    });
    started_rx.await.expect("task started");
    task.abort();
    let _ = task.await;

    let event = rx.recv().await.expect("an aborted task must report");
    assert_eq!(
        failed(event),
        (
            "agent_memory".to_string(),
            "sub-1".to_string(),
            ABORTED_ERROR.to_string()
        )
    );
}

/// A run that returned reports its own outcome; the guard must stay silent so
/// the card is not settled twice (or settled as failed after a success).
#[tokio::test]
async fn a_disarmed_guard_sends_nothing() {
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    let mut report = AbortReport::arm(Some(tx), "help", "sub-2");
    report.disarm();
    drop(report);
    assert!(rx.recv().await.is_none(), "disarmed guard must not report");
}

/// No progress sink (a run with no chat attached) is a no-op, not a panic.
#[test]
fn no_sink_is_a_no_op() {
    drop(AbortReport::arm(None, "help", "sub-3"));
}

/// A busy parent can have the progress channel full at the moment of the
/// abort. The report is the only thing that settles the card, so it must wait
/// for room rather than be dropped.
#[tokio::test]
async fn a_full_channel_still_delivers_the_report_once_there_is_room() {
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    tx.send(AgentProgress::TurnStarted)
        .await
        .expect("fill the channel");

    drop(AbortReport::arm(Some(tx), "help", "sub-full"));

    // Drain the event that was filling the channel; the deferred report follows.
    assert!(matches!(rx.recv().await, Some(AgentProgress::TurnStarted)));
    let event = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
        .await
        .expect("deferred report arrives")
        .expect("channel still open");
    assert_eq!(
        failed(event),
        (
            "help".to_string(),
            "sub-full".to_string(),
            ABORTED_ERROR.to_string()
        )
    );
}

fn completed(task_id: &str) -> AgentProgress {
    AgentProgress::SubagentCompleted {
        agent_id: "agent_memory".into(),
        task_id: task_id.into(),
        elapsed_ms: 1,
        iterations: 1,
        output_chars: 2,
        output: "ok".into(),
        usage: None,
        worktree_path: None,
        changed_files: Vec::new(),
        dirty_status: None,
    }
}

/// A late Cancel aborts a run that has already finished and is still waiting
/// to send its completion (the registry cannot tell). The guard must deliver
/// that real completion, not a synthetic failure and not nothing.
#[tokio::test]
async fn an_abort_during_delivery_resends_the_real_outcome() {
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    tx.send(AgentProgress::TurnStarted)
        .await
        .expect("fill the channel");
    let (sending_tx, sending_rx) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let mut report = AbortReport::arm(Some(tx), "agent_memory", "sub-late");
        let _ = sending_tx.send(());
        report.deliver(completed("sub-late")).await; // blocks: channel full
        report.disarm();
    });
    sending_rx.await.expect("task reached delivery");
    tokio::task::yield_now().await;
    task.abort();
    let _ = task.await;

    assert!(matches!(rx.recv().await, Some(AgentProgress::TurnStarted)));
    let event = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
        .await
        .expect("the real outcome arrives")
        .expect("channel open");
    assert!(
        matches!(event, AgentProgress::SubagentCompleted { ref task_id, .. } if task_id == "sub-late"),
        "expected the real completion, got {event:?}"
    );
}

/// Once the terminal event reached the channel, nothing more is reported —
/// even if the task is dropped before it disarms.
#[tokio::test]
async fn a_delivered_outcome_is_not_reported_twice() {
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    let mut report = AbortReport::arm(Some(tx), "agent_memory", "sub-once");
    report.deliver(completed("sub-once")).await;
    drop(report);
    assert!(matches!(
        rx.recv().await,
        Some(AgentProgress::SubagentCompleted { .. })
    ));
    assert!(rx.recv().await.is_none(), "no second, synthetic report");
}
