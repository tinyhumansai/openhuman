//! Event-bus integration: fans the four v1 trigger sources into the
//! background orchestrator.
//!
//! The subscriber observes (it does not consume) — inbound user messages,
//! cron ticks, Composio webhooks, and sub-agent conclusions all keep
//! flowing through their existing handlers. This subscriber simply makes the
//! background orchestrator *aware* of them.

use std::sync::{Arc, Mutex, OnceLock};

use async_trait::async_trait;

use crate::core::event_bus::{subscribe_global, DomainEvent, EventHandler, SubscriptionHandle};

use super::runtime::TriggerOrchestrator;

/// Resettable subscription slot: the handle is dropped (cancelling the
/// subscription) on teardown so a user/workspace switch can re-register
/// against a fresh orchestrator instead of leaking the stale binding.
static SUBSCRIPTION: OnceLock<Mutex<Option<SubscriptionHandle>>> = OnceLock::new();

fn subscription_slot() -> &'static Mutex<Option<SubscriptionHandle>> {
    SUBSCRIPTION.get_or_init(|| Mutex::new(None))
}

/// Domain filter — only the four v1 trigger-source families. `agent`
/// carries sub-agent conclusion events.
const DOMAINS: &[&str] = &["cron", "channel", "composio", "agent"];

/// Forwards relevant bus events into the [`TriggerOrchestrator`].
pub struct SubconsciousTriggerSubscriber {
    orchestrator: Arc<TriggerOrchestrator>,
}

impl SubconsciousTriggerSubscriber {
    pub fn new(orchestrator: Arc<TriggerOrchestrator>) -> Self {
        Self { orchestrator }
    }
}

#[async_trait]
impl EventHandler for SubconsciousTriggerSubscriber {
    fn name(&self) -> &str {
        "subconscious_triggers::ingest"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(DOMAINS)
    }

    async fn handle(&self, event: &DomainEvent) {
        // `ingest` is non-blocking: it normalizes + admits synchronously and
        // spawns the gate task itself, so we never block event dispatch.
        self.orchestrator.ingest(event);
    }
}

/// Register the trigger subscriber. Idempotent — the handle is held so the
/// subscription stays live; re-registering while already subscribed is a no-op.
pub fn register_subconscious_triggers_subscriber(orchestrator: Arc<TriggerOrchestrator>) {
    let mut guard = subscription_slot()
        .lock()
        .expect("subscription slot poisoned");
    if guard.is_some() {
        return;
    }
    match subscribe_global(Arc::new(SubconsciousTriggerSubscriber::new(orchestrator))) {
        Some(handle) => {
            *guard = Some(handle);
            tracing::debug!("[subconscious_triggers:bus] subscriber registered");
        }
        None => {
            tracing::warn!(
                "[subconscious_triggers:bus] event bus not initialized; subscriber not registered"
            );
        }
    }
}

/// Drop the trigger subscriber (cancels the subscription). Used on user/
/// workspace switch teardown so the next bootstrap re-binds cleanly.
pub fn unregister_subconscious_triggers_subscriber() {
    if subscription_slot()
        .lock()
        .expect("subscription slot poisoned")
        .take()
        .is_some()
    {
        tracing::debug!("[subconscious_triggers:bus] subscriber unregistered");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::config::schema::SubconsciousMode;
    use crate::openhuman::subconscious::LongLivedSession;
    use crate::openhuman::subconscious_triggers::{OrchestratorConfig, TriggerOrchestrator};
    use std::path::PathBuf;

    #[test]
    fn subscriber_watches_v1_domains_only() {
        let session = Arc::new(LongLivedSession::new(
            PathBuf::from("/tmp/ws"),
            SubconsciousMode::Simple,
        ));
        let orch = Arc::new(TriggerOrchestrator::new(
            session,
            OrchestratorConfig::default(),
        ));
        let sub = SubconsciousTriggerSubscriber::new(orch);
        assert_eq!(sub.name(), "subconscious_triggers::ingest");
        assert_eq!(
            sub.domains(),
            Some(["cron", "channel", "composio", "agent"].as_slice())
        );
    }
}
