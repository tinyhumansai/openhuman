//! Subscriber handles and the [`EventHandler`] trait.
//!
//! Provides both a trait-based approach ([`EventHandler`]) for structured
//! handlers and a closure-based shorthand ([`FnSubscriber`]) for simple cases.

use super::events::DomainEvent;
use async_trait::async_trait;
use tokio::task::JoinHandle;

/// Trait for typed event handlers. Implement this to react to domain events.
#[async_trait]
pub trait EventHandler: Send + Sync + 'static {
    /// Human-readable name for logging and diagnostics.
    fn name(&self) -> &str;

    /// Optional domain filter. Return `None` to receive all events,
    /// or `Some(&["agent", "cron"])` to receive only matching domains.
    fn domains(&self) -> Option<&[&str]> {
        None
    }

    /// Handle a single event. Implementations must not block the tokio runtime.
    async fn handle(&self, event: &DomainEvent);
}

/// Opaque handle to a running subscriber task.
///
/// Dropping the handle cancels the subscriber by aborting its background task.
pub struct SubscriptionHandle {
    task: JoinHandle<()>,
    name: String,
}

impl SubscriptionHandle {
    pub(crate) fn new(name: String, task: JoinHandle<()>) -> Self {
        Self { task, name }
    }

    /// Returns the subscriber's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Explicitly cancel the subscriber.
    pub fn cancel(self) {
        tracing::debug!(subscriber = self.name, "[event_bus] cancelling subscriber");
        self.task.abort();
    }
}

impl Drop for SubscriptionHandle {
    fn drop(&mut self) {
        if !self.task.is_finished() {
            tracing::debug!(
                subscriber = self.name,
                "[event_bus] subscriber dropped, aborting task"
            );
            self.task.abort();
        }
    }
}

/// Closure-based subscriber that wraps an `Fn(&DomainEvent)` for simple cases.
///
/// Use [`EventBus::on`] to create one without implementing [`EventHandler`].
pub(crate) struct FnSubscriber<F>
where
    F: Fn(&DomainEvent) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>>
        + Send
        + Sync
        + 'static,
{
    pub(crate) name: String,
    pub(crate) handler: F,
}

#[async_trait]
impl<F> EventHandler for FnSubscriber<F>
where
    F: Fn(&DomainEvent) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>>
        + Send
        + Sync
        + 'static,
{
    fn name(&self) -> &str {
        &self.name
    }

    async fn handle(&self, event: &DomainEvent) {
        (self.handler)(event).await;
    }
}
