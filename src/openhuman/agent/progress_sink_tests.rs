use super::*;

fn sink() -> (ProgressSink, tokio::sync::mpsc::Receiver<AgentProgress>) {
    tokio::sync::mpsc::channel(8)
}

#[tokio::test]
async fn current_returns_none_outside_scope() {
    assert!(current_progress_sink().is_none());
}

#[tokio::test]
async fn with_progress_sink_scopes_and_unscopes() {
    let (tx, mut rx) = sink();

    let observed = with_progress_sink(tx, async { current_progress_sink() }).await;
    let observed = observed.expect("a sink is visible inside the scope");
    observed
        .send(AgentProgress::TurnStarted)
        .await
        .expect("the scoped sink is the caller's live channel");
    assert!(matches!(rx.recv().await, Some(AgentProgress::TurnStarted)));

    // The scope does not leak past its future.
    assert!(current_progress_sink().is_none());
}

#[tokio::test]
async fn nested_scope_yields_the_inner_sink() {
    let (outer, mut outer_rx) = sink();
    let (inner, mut inner_rx) = sink();

    let observed = with_progress_sink(outer, async {
        with_progress_sink(inner, async { current_progress_sink() }).await
    })
    .await
    .expect("a sink is visible inside the nested scope");

    observed
        .send(AgentProgress::TurnCompleted { iterations: 1 })
        .await
        .expect("send on the innermost sink");
    assert!(matches!(
        inner_rx.recv().await,
        Some(AgentProgress::TurnCompleted { iterations: 1 })
    ));
    // The outer sink saw nothing — the inner scope shadows it.
    assert!(outer_rx.try_recv().is_err());
}
