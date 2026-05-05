use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, OnceLock,
};

use async_trait::async_trait;

use crate::core::event_bus::{DomainEvent, EventHandler, SubscriptionHandle};

/// Holds the single process-lifetime subscription handle so it is never
/// double-registered and never dropped (which would abort the task).
static RESTART_HANDLE: OnceLock<SubscriptionHandle> = OnceLock::new();

/// Same idea as [`RESTART_HANDLE`] but for the shutdown subscriber.
static SHUTDOWN_HANDLE: OnceLock<SubscriptionHandle> = OnceLock::new();

/// Register the [`RestartSubscriber`] on the global event bus.
///
/// Idempotent: subsequent calls return immediately if the subscriber is already
/// registered. Owned by the service domain — called from the shared subscriber
/// bootstrap so jsonrpc.rs stays transport-focused.
pub fn register_restart_subscriber() {
    if RESTART_HANDLE.get().is_some() {
        return;
    }

    match crate::core::event_bus::subscribe_global(Arc::new(RestartSubscriber)) {
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

/// Register the [`ShutdownSubscriber`] on the global event bus.
///
/// Mirrors [`register_restart_subscriber`] — idempotent, owned by the service
/// domain, called from the shared subscriber bootstrap in `jsonrpc.rs`.
pub fn register_shutdown_subscriber() {
    if SHUTDOWN_HANDLE.get().is_some() {
        return;
    }

    match crate::core::event_bus::subscribe_global(Arc::new(ShutdownSubscriber)) {
        Some(handle) => {
            let _ = SHUTDOWN_HANDLE.set(handle);
        }
        None => {
            log::warn!("[event_bus] failed to register shutdown subscriber — bus not initialized");
        }
    }
}

/// One-shot gate so only the first restart event actually spawns a replacement
/// process. Subsequent events are ignored (the process is about to exit).
static RESTART_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

/// Same one-shot gate but for shutdown — only the first request actually
/// schedules `process::exit`.
static SHUTDOWN_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

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

/// Long-lived event-bus subscriber that turns shutdown requests into a
/// graceful `process::exit(0)` after a short flush window.
///
/// Distinct from [`RestartSubscriber`]: no replacement process is spawned —
/// we just exit. Frontends that want the process back up are expected to
/// invoke `service.start` (or rely on a supervisor) after calling
/// `service.shutdown`.
pub struct ShutdownSubscriber;

#[async_trait]
impl EventHandler for ShutdownSubscriber {
    fn name(&self) -> &str {
        "service::shutdown"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["system"])
    }

    async fn handle(&self, event: &DomainEvent) {
        let DomainEvent::SystemShutdownRequested { source, reason } = event else {
            return;
        };

        if SHUTDOWN_IN_PROGRESS
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            log::debug!(
                "[service:shutdown] ignoring duplicate shutdown request source={} (already in progress)",
                source,
            );
            return;
        }

        log::warn!(
            "[service:shutdown] executing shutdown source={} reason={}",
            source,
            reason
        );

        // Brief 150ms grace period before exit, mirroring the restart path,
        // so in-flight RPC responses and log writes can flush before the
        // tokio runtime is torn down.
        tokio::spawn(async {
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
            std::process::exit(0);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // NOTE: We deliberately do NOT test the success path of `handle()`
    // for `SystemRestartRequested` — it spawns a tokio task that calls
    // `std::process::exit(0)` after 150ms and would terminate the test
    // runner. We exercise the observable metadata plus the two quick
    // early-return branches instead.

    #[test]
    fn restart_subscriber_name_is_namespaced() {
        assert_eq!(RestartSubscriber.name(), "service::restart");
    }

    #[test]
    fn restart_subscriber_domain_filter_is_system() {
        assert_eq!(RestartSubscriber.domains(), Some(&["system"][..]));
    }

    #[tokio::test]
    async fn handle_returns_early_on_non_restart_event() {
        // A domain event from a different module must be ignored —
        // `handle()` checks the variant and returns without touching
        // RESTART_IN_PROGRESS or spawning a restart.
        RestartSubscriber
            .handle(&DomainEvent::AgentTurnStarted {
                session_id: "s".into(),
                channel: "web".into(),
            })
            .await;
    }

    #[tokio::test]
    async fn handle_ignores_duplicate_restart_when_gate_is_set() {
        // Simulate "a restart is already underway" by flipping the
        // global gate manually. `handle()` must notice this, log, and
        // return without calling into `trigger_self_restart_now`
        // (which would spawn a replacement process).
        let previous = RESTART_IN_PROGRESS.swap(true, Ordering::SeqCst);
        RestartSubscriber
            .handle(&DomainEvent::SystemRestartRequested {
                source: "test".into(),
                reason: "duplicate-suppression".into(),
            })
            .await;
        // Restore the prior gate value so other tests in the same
        // binary aren't skewed by this one.
        RESTART_IN_PROGRESS.store(previous, Ordering::SeqCst);
    }

    #[tokio::test]
    async fn register_restart_subscriber_is_idempotent_and_safe_without_bus() {
        // `subscribe_global` reaches into a tokio broadcast channel, so a
        // runtime must be present — hence `#[tokio::test]`. When the event
        // bus isn't initialised in the test process the first call logs a
        // warning and returns; subsequent calls must also be no-ops rather
        // than registering duplicates.
        register_restart_subscriber();
        register_restart_subscriber();
    }

    // Shutdown subscriber: same shape of metadata + early-return tests as the
    // restart subscriber. The success path (`handle()` → `process::exit`) is
    // intentionally untested for the same reason — it would terminate the
    // test runner.

    #[test]
    fn shutdown_subscriber_name_is_namespaced() {
        assert_eq!(ShutdownSubscriber.name(), "service::shutdown");
    }

    #[test]
    fn shutdown_subscriber_domain_filter_is_system() {
        assert_eq!(ShutdownSubscriber.domains(), Some(&["system"][..]));
    }

    #[tokio::test]
    async fn shutdown_handle_returns_early_on_non_shutdown_event() {
        ShutdownSubscriber
            .handle(&DomainEvent::AgentTurnStarted {
                session_id: "s".into(),
                channel: "web".into(),
            })
            .await;
    }

    #[tokio::test]
    async fn shutdown_handle_ignores_duplicate_when_gate_is_set() {
        let previous = SHUTDOWN_IN_PROGRESS.swap(true, Ordering::SeqCst);
        ShutdownSubscriber
            .handle(&DomainEvent::SystemShutdownRequested {
                source: "test".into(),
                reason: "duplicate-suppression".into(),
            })
            .await;
        SHUTDOWN_IN_PROGRESS.store(previous, Ordering::SeqCst);
    }

    #[tokio::test]
    async fn register_shutdown_subscriber_is_idempotent_and_safe_without_bus() {
        register_shutdown_subscriber();
        register_shutdown_subscriber();
    }
}
