//! Core graceful-shutdown orchestration for the service domain.
//!
//! Mirrors [`super::restart`] but exits the running core process instead of
//! respawning it. RPC/CLI callers acknowledge the request and publish an
//! event; a long-lived subscriber performs the actual `process::exit`. The
//! split keeps the in-process trigger paths (RPC, CLI, internal) sharing one
//! shutdown execution path with the same logging.

use serde::Serialize;

use crate::core::event_bus::{self, DomainEvent};
use crate::rpc::RpcOutcome;

/// JSON-serializable acknowledgement returned to CLI / JSON-RPC callers
/// before the current process exits.
#[derive(Debug, Clone, Serialize)]
pub struct ShutdownStatus {
    pub accepted: bool,
    pub source: String,
    pub reason: String,
}

/// Accepts a shutdown request and publishes it to the global event bus.
///
/// Does not exit directly — the work is performed by
/// [`super::bus::ShutdownSubscriber`] so every in-process trigger uses the
/// same execution path.
pub async fn service_shutdown(
    source: Option<String>,
    reason: Option<String>,
) -> Result<RpcOutcome<ShutdownStatus>, String> {
    let source = source
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "jsonrpc".to_string());
    let reason = reason
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "service.shutdown".to_string());

    event_bus::init_global(event_bus::DEFAULT_CAPACITY);
    log::info!(
        "[service:shutdown] accepted shutdown request source={} reason={}",
        source,
        reason
    );
    event_bus::publish_global(DomainEvent::SystemShutdownRequested {
        source: source.clone(),
        reason: reason.clone(),
    });

    Ok(RpcOutcome::single_log(
        ShutdownStatus {
            accepted: true,
            source,
            reason,
        },
        "service shutdown requested",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Arc;
    use tokio::sync::mpsc;
    use tokio::time::{timeout, Duration};

    struct ShutdownProbe {
        tx: mpsc::UnboundedSender<(String, String)>,
        /// Only forward events whose `source` matches. The event bus is a
        /// process-global singleton (`event_bus::init_global` is idempotent),
        /// so parallel tests in this module publish onto the *same* bus and
        /// every probe sees every shutdown event. Filtering by source keeps
        /// each test isolated to the event it actually published instead of
        /// racing on whichever fires first.
        expected_source: &'static str,
    }

    #[async_trait]
    impl crate::core::event_bus::EventHandler for ShutdownProbe {
        fn name(&self) -> &str {
            "service::shutdown_probe"
        }

        fn domains(&self) -> Option<&[&str]> {
            Some(&["system"])
        }

        async fn handle(&self, event: &DomainEvent) {
            if let DomainEvent::SystemShutdownRequested { source, reason } = event {
                if source == self.expected_source {
                    let _ = self.tx.send((source.clone(), reason.clone()));
                }
            }
        }
    }

    #[tokio::test]
    async fn service_shutdown_publishes_event() {
        let bus = event_bus::init_global(event_bus::DEFAULT_CAPACITY);
        let (tx, mut rx) = mpsc::unbounded_channel();
        let handle = bus.subscribe(Arc::new(ShutdownProbe {
            tx,
            expected_source: "test",
        }));

        let outcome = service_shutdown(Some("test".into()), Some("integration".into()))
            .await
            .expect("shutdown request should succeed");
        assert!(outcome.value.accepted);

        let event = timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("shutdown event should arrive")
            .expect("probe channel should stay open");
        assert_eq!(event.0, "test");
        assert_eq!(event.1, "integration");

        handle.cancel();
    }

    #[tokio::test]
    async fn service_shutdown_defaults_source_and_reason() {
        event_bus::init_global(event_bus::DEFAULT_CAPACITY);
        let outcome = service_shutdown(None, None)
            .await
            .expect("shutdown should succeed");
        assert!(outcome.value.accepted);
        assert_eq!(outcome.value.source, "jsonrpc");
        assert_eq!(outcome.value.reason, "service.shutdown");
    }

    #[tokio::test]
    async fn service_shutdown_trims_whitespace_and_falls_back_for_empty() {
        event_bus::init_global(event_bus::DEFAULT_CAPACITY);
        let outcome = service_shutdown(Some("  ui  ".into()), Some("  ".into()))
            .await
            .expect("shutdown should succeed");
        assert_eq!(outcome.value.source, "ui");
        assert_eq!(outcome.value.reason, "service.shutdown");
    }
}
