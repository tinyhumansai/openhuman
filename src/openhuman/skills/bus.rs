//! Event bus handlers for the skills domain.
//!
//! Keeps skill lifecycle side effects behind the message bus so callers can
//! stop skills without importing cron/webhook cleanup concerns directly.

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;

use crate::openhuman::event_bus::{DomainEvent, EventHandler, SubscriptionHandle};

static SKILL_CLEANUP_HANDLE: OnceLock<SubscriptionHandle> = OnceLock::new();

/// Register the long-lived skill cleanup subscriber on the global event bus.
pub fn register_skill_cleanup_subscriber() {
    if SKILL_CLEANUP_HANDLE.get().is_some() {
        return;
    }

    match crate::openhuman::event_bus::subscribe_global(Arc::new(SkillCleanupSubscriber)) {
        Some(handle) => {
            let _ = SKILL_CLEANUP_HANDLE.set(handle);
        }
        None => {
            log::warn!(
                "[event_bus] failed to register skill cleanup subscriber — bus not initialized"
            );
        }
    }
}

pub struct SkillCleanupSubscriber;

#[async_trait]
impl EventHandler for SkillCleanupSubscriber {
    fn name(&self) -> &str {
        "skill::cleanup"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["skill"])
    }

    async fn handle(&self, event: &DomainEvent) {
        let DomainEvent::SkillStopped { skill_id } = event else {
            return;
        };

        let Some(engine) = crate::openhuman::skills::global_engine() else {
            log::warn!(
                "[skills:bus] received SkillStopped for '{}' but runtime engine is unavailable",
                skill_id
            );
            return;
        };

        log::debug!(
            "[skills:bus] handling SkillStopped cleanup for '{}'",
            skill_id
        );
        engine.cron_scheduler().unregister_all_for_skill(skill_id);
        engine.webhook_router().unregister_skill(skill_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn ignores_non_skill_stop_events() {
        let sub = SkillCleanupSubscriber;
        sub.handle(&DomainEvent::SkillLoaded {
            skill_id: "gmail".into(),
            runtime: "qjs".into(),
        })
        .await;
    }
}
