use super::*;

#[test]
fn subscriber_name_and_domain() {
    let sub = TreeSummarizerEventSubscriber::new();
    assert_eq!(sub.name(), "tree_summarizer::events");
    assert_eq!(sub.domains(), Some(&["tree_summarizer"][..]));
}

#[tokio::test]
async fn handles_hour_completed_without_panic() {
    let sub = TreeSummarizerEventSubscriber::new();
    sub.handle(
        &crate::core::events::DomainEvent::TreeSummarizerHourCompleted {
            namespace: "test".into(),
            node_id: "2024/03/15/14".into(),
            token_count: 500,
        },
    )
    .await;
}

#[tokio::test]
async fn handles_propagated_without_panic() {
    let sub = TreeSummarizerEventSubscriber::new();
    sub.handle(
        &crate::core::events::DomainEvent::TreeSummarizerPropagated {
            namespace: "test".into(),
            node_id: "2024/03/15".into(),
            level: "day".into(),
            token_count: 1500,
        },
    )
    .await;
}

#[tokio::test]
async fn handles_rebuild_without_panic() {
    let sub = TreeSummarizerEventSubscriber::new();
    sub.handle(
        &crate::core::events::DomainEvent::TreeSummarizerRebuildCompleted {
            namespace: "test".into(),
            total_nodes: 42,
        },
    )
    .await;
}

#[tokio::test]
async fn ignores_unrelated_events() {
    let sub = TreeSummarizerEventSubscriber::new();
    sub.handle(&DomainEvent::CronJobTriggered {
        job_id: "j1".into(),
        job_name: "test-job".into(),
        job_type: "shell".into(),
    })
    .await;
    // No panic = pass
}
