//! Event bus handlers for the `flows::` domain.
//!
//! B1 scope: this subscriber only **observes** and logs events that will later
//! drive automatic flow runs (B2). It does not yet map a matched event onto an
//! enabled [`crate::openhuman::flows::Flow`] or call `flows_run` — that bridge,
//! plus `TrustedAutomationSource::Workflow` for externally-triggered runs, is
//! B2+ (see `my_docs/ohxtf/b1-engine-seam-domain/07-execution-and-hitl.md`).

use crate::core::event_bus::{DomainEvent, EventHandler};
use async_trait::async_trait;

/// Listens for events that a saved flow's trigger node might match
/// (`cron`, `composio`, `system` domains) and logs them.
///
/// A future revision resolves matched events to enabled flows and invokes
/// `flows::ops::flows_run` for each match.
pub struct FlowTriggerSubscriber;

impl FlowTriggerSubscriber {
    pub fn new() -> Self {
        Self
    }
}

impl Default for FlowTriggerSubscriber {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl EventHandler for FlowTriggerSubscriber {
    fn name(&self) -> &str {
        "flows::trigger"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["cron", "composio", "system"])
    }

    async fn handle(&self, event: &DomainEvent) {
        // B1: log-only. Once trigger binding lands (B2), this maps the event
        // onto enabled flows whose trigger node kind/config matches it, and
        // runs each match through `flows::ops::flows_run`.
        tracing::debug!(
            target: "flows",
            ?event,
            "[flows] trigger subscriber observed event (B1: not yet dispatched to flows)"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_and_domains_are_stable() {
        let sub = FlowTriggerSubscriber::new();
        assert_eq!(sub.name(), "flows::trigger");
        assert_eq!(sub.domains(), Some(&["cron", "composio", "system"][..]));
    }

    #[tokio::test]
    async fn handle_does_not_panic_on_arbitrary_events() {
        let sub = FlowTriggerSubscriber;
        sub.handle(&DomainEvent::CronJobTriggered {
            job_id: "j1".into(),
            job_name: "test".into(),
            job_type: "shell".into(),
        })
        .await;
    }

    #[test]
    fn default_constructs_the_same_as_new() {
        let a = FlowTriggerSubscriber::new();
        let b = FlowTriggerSubscriber::default();
        assert_eq!(a.name(), b.name());
    }
}
