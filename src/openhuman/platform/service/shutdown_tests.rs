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
impl tinybus::EventHandler<crate::core::events::DomainEvent> for ShutdownProbe {
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
    crate::core::bus::init().await.expect("bus init");
    let bus = crate::core::bus::BUS.get().expect("bus initialised");
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
    let _ = crate::core::bus::init().await;
    let outcome = service_shutdown(None, None)
        .await
        .expect("shutdown should succeed");
    assert!(outcome.value.accepted);
    assert_eq!(outcome.value.source, "jsonrpc");
    assert_eq!(outcome.value.reason, "service.shutdown");
}

#[tokio::test]
async fn service_shutdown_trims_whitespace_and_falls_back_for_empty() {
    let _ = crate::core::bus::init().await;
    let outcome = service_shutdown(Some("  ui  ".into()), Some("  ".into()))
        .await
        .expect("shutdown should succeed");
    assert_eq!(outcome.value.source, "ui");
    assert_eq!(outcome.value.reason, "service.shutdown");
}
