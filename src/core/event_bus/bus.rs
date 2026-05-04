//! Core event bus built on `tokio::sync::broadcast`.
//!
//! The [`EventBus`] is a **singleton** — one instance handles all events for
//! the entire application. Call [`init_global`] once at startup, then use
//! [`publish_global`], [`subscribe_global`], and [`global`] from anywhere.
//!
//! For typed request/response calls between modules, see the parallel
//! [`super::native_request`] surface — in-process Rust-typed dispatch that
//! passes trait objects and channels through unchanged (no serialization).

use super::events::DomainEvent;
use super::native_request::init_native_registry;
use super::subscriber::{EventHandler, FnSubscriber, SubscriptionHandle};
use futures::FutureExt;
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, OnceLock};
use tokio::sync::broadcast;

/// Global event bus instance, initialized once at startup.
static GLOBAL_BUS: OnceLock<EventBus> = OnceLock::new();

/// Default broadcast channel capacity.
pub const DEFAULT_CAPACITY: usize = 256;

// ── Global singleton API ────────────────────────────────────────────────

/// Initialize the global event bus. Must be called **once** during startup.
///
/// This function:
/// 1. Initializes the native request registry.
/// 2. Sets up the global singleton bus with the specified capacity.
///
/// Subsequent calls return the already-initialized bus without changing
/// its capacity. This ensures thread-safe, consistent initialization.
///
/// # Arguments
///
/// * `capacity` - The maximum number of buffered events for the broadcast channel.
pub fn init_global(capacity: usize) -> &'static EventBus {
    // Initialize the native request registry first so handler registration
    // is always safe from anywhere in the process once the bus is up.
    init_native_registry();
    GLOBAL_BUS.get_or_init(|| {
        tracing::debug!(capacity, "[event_bus] initializing global singleton");
        EventBus::create(capacity)
    })
}

/// Get the global event bus.
///
/// Returns `Some(&EventBus)` if [`init_global`] has been called, otherwise `None`.
pub fn global() -> Option<&'static EventBus> {
    GLOBAL_BUS.get()
}

/// Publish an event on the global bus.
///
/// This is the primary way to notify other modules about domain events
/// (e.g., an agent turn completed, a memory was stored).
///
/// # Arguments
///
/// * `event` - The [`DomainEvent`] to broadcast to all subscribers.
pub fn publish_global(event: DomainEvent) {
    if let Some(bus) = GLOBAL_BUS.get() {
        bus.publish(event);
    } else {
        tracing::trace!("[event_bus] global bus not initialized, dropping event");
    }
}

/// Subscribe a handler on the global bus.
///
/// The handler will receive all events that match its domain filter.
/// Returns a [`SubscriptionHandle`] that will cancel the subscription when dropped.
///
/// # Arguments
///
/// * `handler` - An implementation of the [`EventHandler`] trait.
pub fn subscribe_global(handler: Arc<dyn EventHandler>) -> Option<SubscriptionHandle> {
    GLOBAL_BUS.get().map(|bus| bus.subscribe(handler))
}

// ── EventBus struct ─────────────────────────────────────────────────────

/// The event bus, wrapping a `tokio::sync::broadcast` channel.
///
/// It provides a many-to-many communication channel for [`DomainEvent`]s.
/// There is exactly **one** production instance at runtime (the global singleton).
#[derive(Clone, Debug)]
pub struct EventBus {
    /// The sending end of the broadcast channel.
    tx: broadcast::Sender<DomainEvent>,
}

impl EventBus {
    /// Create a new event bus with the given capacity.
    ///
    /// This is used internally by [`init_global`] and by tests for isolation.
    pub(crate) fn create(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity.max(1));
        Self { tx }
    }

    /// Publish an event to all active subscribers.
    ///
    /// The event is cloned and sent to each subscriber's receiving end.
    /// If no subscribers are currently listening, the event is silently dropped.
    pub fn publish(&self, event: DomainEvent) {
        let receiver_count = self.tx.receiver_count();
        tracing::debug!(
            domain = event.domain(),
            receivers = receiver_count,
            "[event_bus] publishing {:?}",
            std::mem::discriminant(&event)
        );
        let _ = self.tx.send(event);
    }

    /// Subscribe with a trait-based [`EventHandler`].
    ///
    /// Spawns a background task that listens for events and dispatches them
    /// to the handler's `handle` method.
    ///
    /// # Arguments
    ///
    /// * `handler` - The handler to register. Its `domains()` filter is checked
    ///   before every dispatch.
    ///
    /// # Returns
    ///
    /// A [`SubscriptionHandle`] to manage the lifetime of the background task.
    pub fn subscribe(&self, handler: Arc<dyn EventHandler>) -> SubscriptionHandle {
        let mut rx = self.tx.subscribe();
        let name = handler.name().to_string();
        let domains: Option<Vec<String>> = handler
            .domains()
            .map(|d| d.iter().map(|s| s.to_string()).collect());

        tracing::debug!(
            subscriber = name,
            domains = ?domains,
            "[event_bus] registering subscriber"
        );

        let name_for_task = name.clone();
        let task = tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(event) => {
                        // Apply domain filter: only dispatch if the event domain
                        // matches one of the subscriber's allowed domains.
                        if let Some(ref allowed) = domains {
                            if !allowed.iter().any(|d| d == event.domain()) {
                                continue;
                            }
                        }
                        tracing::trace!(
                            handler = handler.name(),
                            domain = event.domain(),
                            "[event_bus] dispatching to handler"
                        );
                        // Wrap the handler call in AssertUnwindSafe so that a
                        // panic in one handler doesn't crash the entire event loop.
                        let result = AssertUnwindSafe(handler.handle(&event))
                            .catch_unwind()
                            .await;
                        if let Err(panic) = result {
                            let msg = panic
                                .downcast_ref::<&str>()
                                .copied()
                                .or_else(|| panic.downcast_ref::<String>().map(|s| s.as_str()))
                                .unwrap_or("unknown panic");
                            tracing::error!(
                                handler = name_for_task,
                                domain = event.domain(),
                                panic = msg,
                                "[event_bus] handler panicked, continuing"
                            );
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(
                            handler = name_for_task,
                            skipped = n,
                            "[event_bus] subscriber lagged, skipped events"
                        );
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        tracing::info!(
                            handler = name_for_task,
                            "[event_bus] channel closed, subscriber exiting"
                        );
                        break;
                    }
                }
            }
        });

        SubscriptionHandle::new(name, task)
    }

    /// Subscribe with an async closure.
    ///
    /// This is a convenience method for simple, one-off event handlers.
    /// It doesn't support domain filtering directly; the closure will receive
    /// every event published on the bus.
    pub fn on<F>(&self, name: &str, handler: F) -> SubscriptionHandle
    where
        F: Fn(&DomainEvent) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>>
            + Send
            + Sync
            + 'static,
    {
        let subscriber = Arc::new(FnSubscriber {
            name: name.to_string(),
            handler,
        });
        self.subscribe(subscriber)
    }

    /// Returns the current number of active subscribers (receivers).
    pub fn subscriber_count(&self) -> usize {
        self.tx.receiver_count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::time::{sleep, Duration};

    /// Tests use `EventBus::create()` for isolation — each test gets its own
    /// bus so they don't interfere with each other or the global singleton.

    #[tokio::test]
    async fn publish_without_subscribers_does_not_panic() {
        let bus = EventBus::create(16);
        bus.publish(DomainEvent::SystemStartup {
            component: "test".into(),
        });
    }

    #[tokio::test]
    async fn single_subscriber_receives_event() {
        let bus = EventBus::create(16);
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = Arc::clone(&counter);

        let _handle = bus.on("test-sub", move |_event| {
            let c = Arc::clone(&counter_clone);
            Box::pin(async move {
                c.fetch_add(1, Ordering::SeqCst);
            })
        });

        bus.publish(DomainEvent::SystemStartup {
            component: "test".into(),
        });

        sleep(Duration::from_millis(50)).await;
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn multiple_subscribers_receive_same_event() {
        let bus = EventBus::create(16);
        let counter = Arc::new(AtomicUsize::new(0));

        let c1 = Arc::clone(&counter);
        let _h1 = bus.on("sub-1", move |_| {
            let c = Arc::clone(&c1);
            Box::pin(async move {
                c.fetch_add(1, Ordering::SeqCst);
            })
        });

        let c2 = Arc::clone(&counter);
        let _h2 = bus.on("sub-2", move |_| {
            let c = Arc::clone(&c2);
            Box::pin(async move {
                c.fetch_add(1, Ordering::SeqCst);
            })
        });

        bus.publish(DomainEvent::SystemStartup {
            component: "test".into(),
        });

        sleep(Duration::from_millis(50)).await;
        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn domain_filtering_works() {
        use super::super::subscriber::EventHandler;

        struct CronOnlyHandler {
            counter: Arc<AtomicUsize>,
        }

        #[async_trait::async_trait]
        impl EventHandler for CronOnlyHandler {
            fn name(&self) -> &str {
                "cron-only"
            }
            fn domains(&self) -> Option<&[&str]> {
                Some(&["cron"])
            }
            async fn handle(&self, _event: &DomainEvent) {
                self.counter.fetch_add(1, Ordering::SeqCst);
            }
        }

        let bus = EventBus::create(16);
        let counter = Arc::new(AtomicUsize::new(0));

        let _handle = bus.subscribe(Arc::new(CronOnlyHandler {
            counter: Arc::clone(&counter),
        }));

        // This should be filtered out (domain = "system")
        bus.publish(DomainEvent::SystemStartup {
            component: "test".into(),
        });

        // This should pass through (domain = "cron")
        bus.publish(DomainEvent::CronJobTriggered {
            job_id: "j1".into(),
            job_name: "test-job".into(),
            job_type: "shell".into(),
        });

        sleep(Duration::from_millis(50)).await;
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn handle_drop_cancels_subscriber() {
        let bus = EventBus::create(16);
        let counter = Arc::new(AtomicUsize::new(0));
        let c = Arc::clone(&counter);

        let handle = bus.on("drop-test", move |_| {
            let c = Arc::clone(&c);
            Box::pin(async move {
                c.fetch_add(1, Ordering::SeqCst);
            })
        });

        assert_eq!(bus.subscriber_count(), 1);
        drop(handle);
        sleep(Duration::from_millis(20)).await;
        assert_eq!(bus.subscriber_count(), 0);
    }

    #[tokio::test]
    async fn subscriber_count_tracks_correctly() {
        let bus = EventBus::create(16);
        assert_eq!(bus.subscriber_count(), 0);

        let h1 = bus.on("s1", |_| Box::pin(async {}));
        assert_eq!(bus.subscriber_count(), 1);

        let h2 = bus.on("s2", |_| Box::pin(async {}));
        assert_eq!(bus.subscriber_count(), 2);

        drop(h1);
        sleep(Duration::from_millis(20)).await;
        assert_eq!(bus.subscriber_count(), 1);

        drop(h2);
        sleep(Duration::from_millis(20)).await;
        assert_eq!(bus.subscriber_count(), 0);
    }
}
