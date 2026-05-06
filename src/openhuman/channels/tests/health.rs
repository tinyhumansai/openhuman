use super::super::commands::{classify_health_result, ChannelHealthState};
use super::super::runtime::spawn_supervised_listener;
use super::super::{traits, Channel};
use super::common::AlwaysFailChannel;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

fn classify_health_ok_true() {
    let state = classify_health_result(&Ok(true));
    assert_eq!(state, ChannelHealthState::Healthy);
}

#[test]
fn classify_health_ok_false() {
    let state = classify_health_result(&Ok(false));
    assert_eq!(state, ChannelHealthState::Unhealthy);
}

#[tokio::test]
async fn classify_health_timeout() {
    let result = tokio::time::timeout(Duration::from_millis(1), async {
        tokio::time::sleep(Duration::from_millis(20)).await;
        true
    })
    .await;
    let state = classify_health_result(&result);
    assert_eq!(state, ChannelHealthState::Timeout);
}

#[tokio::test]
async fn supervised_listener_marks_error_and_restarts_on_failures() {
    let calls = Arc::new(AtomicUsize::new(0));
    let name = Box::leak(format!("test-supervised-fail-{}", uuid::Uuid::new_v4()).into_boxed_str());
    let channel: Arc<dyn Channel> = Arc::new(AlwaysFailChannel {
        name,
        calls: Arc::clone(&calls),
    });
    let component_name = format!("channel:{name}");

    let (tx, rx) = tokio::sync::mpsc::channel::<traits::ChannelMessage>(1);
    // The global health subscriber may have been registered by another test
    // runtime; keep a fresh subscriber alive for this test's runtime too.
    crate::core::event_bus::init_global(crate::core::event_bus::DEFAULT_CAPACITY);
    let _health_handle = crate::core::event_bus::subscribe_global(Arc::new(
        crate::openhuman::health::bus::HealthSubscriber,
    ))
    .expect("event bus should be initialized for channel health test");
    tokio::task::yield_now().await;
    let handle = spawn_supervised_listener(channel, tx, 1, 1);

    let component = wait_for_component_error(&component_name).await;
    drop(rx);
    handle.abort();
    let _ = handle.await;

    assert_eq!(component["status"], "error");
    assert!(component["restart_count"].as_u64().unwrap_or(0) >= 1);
    assert!(component["last_error"]
        .as_str()
        .unwrap_or("")
        .contains("listen boom"));
    assert!(calls.load(Ordering::SeqCst) >= 1);
}

async fn wait_for_component_error(component_name: &str) -> serde_json::Value {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        let snapshot = crate::openhuman::health::snapshot_json();
        let component = snapshot["components"][component_name].clone();
        if component["status"] == "error" && component["restart_count"].as_u64().unwrap_or(0) >= 1 {
            return component;
        }

        if tokio::time::Instant::now() >= deadline {
            panic!("timed out waiting for {component_name} to enter error state; last={component}");
        }

        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
