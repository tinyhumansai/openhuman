use super::*;
use async_trait::async_trait;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::time::{timeout, Duration};

static RESTART_TEST_ID: AtomicUsize = AtomicUsize::new(0);

struct RestartProbe {
    tx: mpsc::UnboundedSender<(String, String)>,
}

#[async_trait]
impl tinybus::EventHandler<crate::core::events::DomainEvent> for RestartProbe {
    fn name(&self) -> &str {
        "service::restart_probe"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["system"])
    }

    async fn handle(&self, event: &crate::core::events::DomainEvent) {
        if let crate::core::events::DomainEvent::SystemRestartRequested { source, reason } = event {
            let _ = self.tx.send((source.clone(), reason.clone()));
        }
    }
}

#[tokio::test]
async fn service_restart_publishes_restart_event() {
    crate::core::bus::init().await.expect("bus init");
    let bus = crate::core::bus::BUS.get().expect("bus initialised");
    let (tx, mut rx) = mpsc::unbounded_channel();
    let handle = bus.subscribe(Arc::new(RestartProbe { tx }));
    let id = RESTART_TEST_ID.fetch_add(1, Ordering::SeqCst);
    let source = format!("test-{id}");
    let reason = format!("integration-{id}");

    let outcome = service_restart(Some(source.clone()), Some(reason.clone()))
        .await
        .expect("restart request should succeed");
    assert!(outcome.value.accepted);

    let event = timeout(Duration::from_secs(1), async {
        loop {
            let event = rx.recv().await.expect("probe channel should stay open");
            if event.0 == source && event.1 == reason {
                break event;
            }
        }
    })
    .await
    .expect("matching restart event should arrive");
    assert_eq!(event.0, source);
    assert_eq!(event.1, reason);

    handle.cancel();
}

#[tokio::test]
async fn service_restart_defaults_source_and_reason() {
    let _ = crate::core::bus::init().await;
    let outcome = service_restart(None, None)
        .await
        .expect("restart should succeed");
    assert!(outcome.value.accepted);
    assert_eq!(outcome.value.source, "jsonrpc");
    assert_eq!(outcome.value.reason, "service.restart");
}

#[tokio::test]
async fn service_restart_trims_whitespace() {
    let _ = crate::core::bus::init().await;
    let outcome = service_restart(Some("  ui  ".into()), Some("  user request  ".into()))
        .await
        .expect("restart should succeed");
    assert_eq!(outcome.value.source, "ui");
    assert_eq!(outcome.value.reason, "user request");
}

#[tokio::test]
async fn service_restart_empty_strings_use_defaults() {
    let _ = crate::core::bus::init().await;
    let outcome = service_restart(Some("".into()), Some("  ".into()))
        .await
        .expect("restart should succeed");
    assert_eq!(outcome.value.source, "jsonrpc");
    assert_eq!(outcome.value.reason, "service.restart");
}

#[test]
fn restart_status_serializes() {
    let status = RestartStatus {
        accepted: true,
        source: "test".into(),
        reason: "testing".into(),
    };
    let json = serde_json::to_string(&status).unwrap();
    assert!(json.contains("\"accepted\":true"));
    assert!(json.contains("\"source\":\"test\""));
}

#[test]
fn apply_startup_restart_delay_from_env_noop_when_unset() {
    // Ensure the env var is not set, then call — should not block
    let _prev = std::env::var(RESTART_DELAY_ENV).ok();
    std::env::remove_var(RESTART_DELAY_ENV);
    apply_startup_restart_delay_from_env(); // should return immediately
}
