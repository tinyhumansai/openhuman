//! Event bus integration for tree_summarizer.
//!
//! Subscribes to `TreeSummarizer*` events and logs them for observability.
//! Future subscribers can react to these events for cross-module workflows.

use crate::core::events::DomainEvent;
use async_trait::async_trait;
use tinybus::EventHandler;

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
impl EventHandler<DomainEvent> for TreeSummarizerEventSubscriber {
    fn name(&self) -> &str {
        "tree_summarizer::events"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["tree_summarizer"])
    }

    async fn handle(&self, event: &DomainEvent) {
        match event {
            crate::core::events::DomainEvent::TreeSummarizerHourCompleted {
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
            crate::core::events::DomainEvent::TreeSummarizerPropagated {
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
            crate::core::events::DomainEvent::TreeSummarizerRebuildCompleted {
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
#[path = "bus_tests.rs"]
mod tests;
