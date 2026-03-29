use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{self, Duration};

use super::engine::{global_engine, AccessibilityEngine, EngineState};
use super::helpers::validate_input_action;
use super::types::{InputActionParams, StartSessionParams};
use crate::openhuman::config::ScreenIntelligenceConfig;

#[test]
fn validates_coordinates_and_actions() {
    let ok = InputActionParams {
        action: "mouse_move".to_string(),
        x: Some(10),
        y: Some(20),
        button: None,
        text: None,
        key: None,
        modifiers: None,
    };
    assert!(validate_input_action(&ok).is_ok());

    let bad = InputActionParams {
        action: "mouse_click".to_string(),
        x: Some(-1),
        y: Some(20),
        button: None,
        text: None,
        key: None,
        modifiers: None,
    };
    assert!(validate_input_action(&bad).is_err());

    let unsupported = InputActionParams {
        action: "open_portal".to_string(),
        x: None,
        y: None,
        button: None,
        text: None,
        key: None,
        modifiers: None,
    };
    assert!(validate_input_action(&unsupported).is_err());
}

#[tokio::test]
async fn session_lifecycle_transitions_and_ttl_expiry() {
    let engine = Arc::new(AccessibilityEngine {
        inner: Mutex::new(EngineState::new(ScreenIntelligenceConfig {
            baseline_fps: 8.0,
            session_ttl_secs: 1,
            denylist: vec!["1password".to_string()],
            ..Default::default()
        })),
    });

    let start = engine
        .start_session(StartSessionParams {
            consent: true,
            ttl_secs: Some(1),
            screen_monitoring: Some(true),
            device_control: Some(true),
            predictive_input: Some(true),
        })
        .await;

    if cfg!(target_os = "macos") {
        if start.is_ok() {
            let active = engine.status().await;
            assert!(active.session.active);
            // ttl_secs is clamped to [30, 3600]; a 1s request becomes 30s wall-clock expiry.
            assert_eq!(active.session.ttl_secs, 30);

            let _ = engine
                .stop_session(Some("test_session_end".to_string()))
                .await;
            let ended = engine.status().await;
            assert!(!ended.session.active);
        }
    } else {
        assert!(start.is_err());
    }
}

#[tokio::test]
async fn panic_stop_behavior_stops_session() {
    if !cfg!(target_os = "macos") {
        return;
    }

    let engine = global_engine();

    let started = engine
        .start_session(StartSessionParams {
            consent: true,
            ttl_secs: Some(60),
            screen_monitoring: Some(true),
            device_control: Some(true),
            predictive_input: Some(true),
        })
        .await;

    if started.is_err() {
        return;
    }

    let result = engine
        .input_action(InputActionParams {
            action: "panic_stop".to_string(),
            x: None,
            y: None,
            button: None,
            text: None,
            key: None,
            modifiers: None,
        })
        .await
        .expect("panic action should return");

    assert!(result.accepted);
    assert!(!engine.status().await.session.active);
}

#[tokio::test]
async fn capture_scheduler_adds_baseline_frames() {
    if !cfg!(target_os = "macos") {
        return;
    }

    let engine = Arc::new(AccessibilityEngine {
        inner: Mutex::new(EngineState::new(ScreenIntelligenceConfig {
            baseline_fps: 6.0,
            session_ttl_secs: 2,
            ..Default::default()
        })),
    });

    let started = engine
        .start_session(StartSessionParams {
            consent: true,
            ttl_secs: Some(2),
            screen_monitoring: Some(true),
            device_control: Some(true),
            predictive_input: Some(true),
        })
        .await;

    if started.is_err() {
        return;
    }

    time::sleep(Duration::from_millis(700)).await;

    let status = engine.status().await;
    assert!(status.session.frames_in_memory >= 1);

    let _ = engine.stop_session(Some("test_end".to_string())).await;
}
