//! Gmail domain event bus subscriber.
//!
//! Subscribes to `CronJobCompleted` events and, when the completing job
//! corresponds to a Gmail sync cron job, triggers a `sync_now` call via
//! the controller registry. Also publishes `GmailMessagesIngested` for
//! downstream consumers.

use async_trait::async_trait;

use crate::core::event_bus::{DomainEvent, EventHandler, SubscriptionHandle};

/// Subscribes to cron completion events for Gmail-related sync jobs.
pub struct GmailCronSubscriber;

#[async_trait]
impl EventHandler for GmailCronSubscriber {
    fn name(&self) -> &str {
        "gmail::cron_subscriber"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["cron", "gmail"])
    }

    async fn handle(&self, event: &DomainEvent) {
        match event {
            DomainEvent::CronJobCompleted { job_id, success } => {
                // We can't efficiently check if this job belongs to Gmail
                // without a DB lookup — keep this handler as a debug log
                // only. The actual sync is driven by the cron command itself
                // (`gmail_sync_now` via the controller registry).
                log::trace!(
                    "[gmail][bus] CronJobCompleted job_id={} success={}",
                    job_id,
                    success
                );
            }
            DomainEvent::GmailMessagesIngested { account_id, count } => {
                log::info!(
                    "[gmail][bus] GmailMessagesIngested account_id={} count={}",
                    account_id,
                    count
                );
            }
            _ => {}
        }
    }
}

/// Register gmail domain event subscribers at startup.
///
/// Returns the `SubscriptionHandle`; drop it to unsubscribe.
pub fn register_subscribers() -> Option<SubscriptionHandle> {
    use std::sync::Arc;
    crate::core::event_bus::subscribe_global(Arc::new(GmailCronSubscriber))
}
