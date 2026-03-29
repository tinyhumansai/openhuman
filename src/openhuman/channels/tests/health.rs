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
    let channel: Arc<dyn Channel> = Arc::new(AlwaysFailChannel {
        name: "test-supervised-fail",
        calls: Arc::clone(&calls),
    });

    let (tx, rx) = tokio::sync::mpsc::channel::<traits::ChannelMessage>(1);
    let handle = spawn_supervised_listener(channel, tx, 1, 1);

    tokio::time::sleep(Duration::from_millis(80)).await;
    drop(rx);
    handle.abort();
    let _ = handle.await;

    let snapshot = crate::openhuman::health::snapshot_json();
    let component = &snapshot["components"]["channel:test-supervised-fail"];
    assert_eq!(component["status"], "error");
    assert!(component["restart_count"].as_u64().unwrap_or(0) >= 1);
    assert!(component["last_error"]
        .as_str()
        .unwrap_or("")
        .contains("listen boom"));
    assert!(calls.load(Ordering::SeqCst) >= 1);
}
