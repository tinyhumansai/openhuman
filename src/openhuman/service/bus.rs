use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, OnceLock,
};

use async_trait::async_trait;

use crate::openhuman::event_bus::{DomainEvent, EventHandler, SubscriptionHandle};

/// Holds the single process-lifetime subscription handle so it is never
/// double-registered and never dropped (which would abort the task).
static RESTART_HANDLE: OnceLock<SubscriptionHandle> = OnceLock::new();

/// Register the [`RestartSubscriber`] on the global event bus.
///
/// Idempotent: subsequent calls return immediately if the subscriber is already
/// registered. Owned by the service domain — called from the shared subscriber
/// bootstrap so jsonrpc.rs stays transport-focused.
pub fn register_restart_subscriber() {
    if RESTART_HANDLE.get().is_some() {
        return;
    }

    match crate::openhuman::event_bus::subscribe_global(Arc::new(RestartSubscriber)) {
        Some(handle) => {
            // Store the handle; OnceLock ensures at most one wins if there is a
            // race between two threads calling this function concurrently.
            let _ = RESTART_HANDLE.set(handle);
        }
        None => {
            log::warn!("[event_bus] failed to register restart subscriber — bus not initialized");
        }
    }
}

/// One-shot gate so only the first restart event actually spawns a replacement
/// process. Subsequent events are ignored (the process is about to exit).
static RESTART_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

/// Long-lived event-bus subscriber that turns restart requests into a real
/// process respawn.
///
/// This subscriber is registered during core bootstrap so any restart
/// request published from RPC, CLI, or another internal component goes through
/// the same execution path.
pub struct RestartSubscriber;

#[async_trait]
impl EventHandler for RestartSubscriber {
    fn name(&self) -> &str {
        "service::restart"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["system"])
    }

    async fn handle(&self, event: &DomainEvent) {
        let DomainEvent::SystemRestartRequested { source, reason } = event else {
            return;
        };

        // Atomically claim the restart slot — only the first caller proceeds.
        if RESTART_IN_PROGRESS
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            log::debug!(
                "[service:restart] ignoring duplicate restart request source={} (restart already in progress)",
                source,
            );
            return;
        }

        log::warn!(
            "[service:restart] executing restart request source={}",
            source
        );

        match crate::openhuman::service::restart::trigger_self_restart_now(source, reason) {
            Ok(child_pid) => {
                log::warn!(
                    "[service:restart] replacement pid={} spawned; exiting current process",
                    child_pid
                );
                // Brief 150ms grace period before exit: allows in-flight log
                // flushes and the replacement process to bind its listener before
                // this process terminates. Empirically tuned — increase if logs
                // are truncated on shutdown.
                tokio::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                    std::process::exit(0);
                });
            }
            Err(err) => {
                log::error!("[service:restart] failed to restart current process: {err}");
                // Reset the gate so a subsequent attempt can try again.
                RESTART_IN_PROGRESS.store(false, Ordering::SeqCst);
            }
        }
    }
}
