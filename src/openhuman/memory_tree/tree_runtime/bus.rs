//! Event bus integration for tree_summarizer.
//!
//! Subscribes to `TreeSummarizer*` events and logs them for observability.
//! Future subscribers can react to these events for cross-module workflows.

use crate::core::event_bus::{DomainEvent, EventHandler};
use async_trait::async_trait;

/// Subscribes to tree summarizer events and logs activity.
pub struct TreeSummarizerEventSubscriber;

impl Default for TreeSummarizerEventSubscriber {
    fn default() -> Self {
        Self::new()
    }
}

impl TreeSummarizerEventSubscriber {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl EventHandler for TreeSummarizerEventSubscriber {
    fn name(&self) -> &str {
        "tree_summarizer::events"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["tree_summarizer"])
    }

    async fn handle(&self, event: &DomainEvent) {
        match event {
            DomainEvent::TreeSummarizerHourCompleted {
                namespace,
                node_id,
                token_count,
            } => {
                tracing::info!(
                    namespace = %namespace,
                    node_id = %node_id,
                    token_count = %token_count,
                    "[tree_summarizer] hour leaf completed"
                );
            }
            DomainEvent::TreeSummarizerPropagated {
                namespace,
                node_id,
                level,
                token_count,
            } => {
                tracing::info!(
                    namespace = %namespace,
                    node_id = %node_id,
                    level = %level,
                    token_count = %token_count,
                    "[tree_summarizer] node propagated"
                );
            }
            DomainEvent::TreeSummarizerRebuildCompleted {
                namespace,
                total_nodes,
            } => {
                tracing::info!(
                    namespace = %namespace,
                    total_nodes = %total_nodes,
                    "[tree_summarizer] tree rebuild completed"
                );
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
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
        sub.handle(&DomainEvent::TreeSummarizerHourCompleted {
            namespace: "test".into(),
            node_id: "2024/03/15/14".into(),
            token_count: 500,
        })
        .await;
    }

    #[tokio::test]
    async fn handles_propagated_without_panic() {
        let sub = TreeSummarizerEventSubscriber::new();
        sub.handle(&DomainEvent::TreeSummarizerPropagated {
            namespace: "test".into(),
            node_id: "2024/03/15".into(),
            level: "day".into(),
            token_count: 1500,
        })
        .await;
    }

    #[tokio::test]
    async fn handles_rebuild_without_panic() {
        let sub = TreeSummarizerEventSubscriber::new();
        sub.handle(&DomainEvent::TreeSummarizerRebuildCompleted {
            namespace: "test".into(),
            total_nodes: 42,
        })
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
}
