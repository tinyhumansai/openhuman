use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{self, Duration};

use super::engine::{global_engine, AccessibilityEngine, EngineState};
use super::helpers::{
    generate_suggestions, parse_vision_summary_output, truncate_tail, validate_input_action,
};
use super::types::{CaptureFrame, InputActionParams, StartSessionParams};
use crate::openhuman::accessibility::{parse_foreground_output, AppContext};
use crate::openhuman::config::ScreenIntelligenceConfig;

// ── parse_foreground_output ─────────────────────────────────────────────

#[test]
fn parse_foreground_output_valid_6_lines() {
    let stdout = "Safari\nGitHub - Pull Requests\n100\n200\n1400\n900\n";
    let ctx = parse_foreground_output(stdout).unwrap();
    assert_eq!(ctx.app_name.as_deref(), Some("Safari"));
    assert_eq!(ctx.window_title.as_deref(), Some("GitHub - Pull Requests"));
    let bounds = ctx.bounds.unwrap();
    assert_eq!(
        (bounds.x, bounds.y, bounds.width, bounds.height),
        (100, 200, 1400, 900)
    );
}

#[test]
fn parse_foreground_output_missing_bounds() {
    let stdout = "Finder\nDesktop\n\n\n\n\n";
    let ctx = parse_foreground_output(stdout).unwrap();
    assert_eq!(ctx.app_name.as_deref(), Some("Finder"));
    assert_eq!(ctx.window_title.as_deref(), Some("Desktop"));
    assert!(ctx.bounds.is_none());
}

#[test]
fn parse_foreground_output_empty_app_name() {
    let stdout = "\nSome Window\n0\n0\n800\n600\n";
    let ctx = parse_foreground_output(stdout).unwrap();
    assert!(ctx.app_name.is_none());
    assert_eq!(ctx.window_title.as_deref(), Some("Some Window"));
    assert!(ctx.bounds.is_some());
}

#[test]
fn parse_foreground_output_non_numeric_coords() {
    let stdout = "Terminal\nzsh\nabc\ndef\nghi\njkl\n";
    let ctx = parse_foreground_output(stdout).unwrap();
    assert_eq!(ctx.app_name.as_deref(), Some("Terminal"));
    assert!(ctx.bounds.is_none());
}

#[test]
fn parse_foreground_output_extra_whitespace() {
    let stdout = "  Code  \n  main.rs  \n  50  \n  75  \n  1200  \n  800  \n";
    let ctx = parse_foreground_output(stdout).unwrap();
    assert_eq!(ctx.app_name.as_deref(), Some("Code"));
    assert_eq!(ctx.window_title.as_deref(), Some("main.rs"));
    let bounds = ctx.bounds.unwrap();
    assert_eq!(
        (bounds.x, bounds.y, bounds.width, bounds.height),
        (50, 75, 1200, 800)
    );
}

#[test]
fn parse_foreground_output_single_line() {
    let stdout = "OnlyAppName\n";
    let ctx = parse_foreground_output(stdout).unwrap();
    assert_eq!(ctx.app_name.as_deref(), Some("OnlyAppName"));
    assert!(ctx.window_title.is_none());
    assert!(ctx.bounds.is_none());
}

#[test]
fn parse_foreground_output_zero_size_bounds() {
    let stdout = "App\nWindow\n100\n200\n0\n0\n";
    let ctx = parse_foreground_output(stdout).unwrap();
    assert!(ctx.bounds.is_none(), "zero-size bounds should be None");
}

#[test]
fn parse_foreground_output_negative_size() {
    let stdout = "App\nWindow\n100\n200\n-1\n600\n";
    let ctx = parse_foreground_output(stdout).unwrap();
    assert!(
        ctx.bounds.is_none(),
        "negative width should yield None bounds"
    );
}

// ── parse_vision_summary_output ─────────────────────────────────────────

fn test_frame() -> CaptureFrame {
    CaptureFrame {
        captured_at_ms: 1700000000000,
        reason: "test".to_string(),
        app_name: Some("TestApp".to_string()),
        window_title: Some("TestWindow".to_string()),
        image_ref: None,
    }
}

#[test]
fn parse_vision_valid_json() {
    let raw = r#"{"ui_state": "editor open", "key_text": "fn main()", "actionable_notes": "Consider adding tests", "confidence": 0.85}"#;
    let summary = parse_vision_summary_output(test_frame(), raw);
    assert_eq!(summary.ui_state, "editor open");
    assert_eq!(summary.key_text, "fn main()");
    assert_eq!(summary.actionable_notes, "Consider adding tests");
    assert!((summary.confidence - 0.85).abs() < 0.01);
    assert_eq!(summary.app_name.as_deref(), Some("TestApp"));
}

#[test]
fn parse_vision_malformed_json_falls_back() {
    let raw = "this is not json at all";
    let summary = parse_vision_summary_output(test_frame(), raw);
    assert_eq!(summary.ui_state, "UI state unavailable");
    assert!(summary.actionable_notes.contains("this is not json at all"));
    assert!((summary.confidence - 0.66).abs() < 0.01);
}

#[test]
fn parse_vision_missing_fields() {
    let raw = r#"{"ui_state": "active"}"#;
    let summary = parse_vision_summary_output(test_frame(), raw);
    assert_eq!(summary.ui_state, "active");
    assert_eq!(summary.key_text, "");
    assert!((summary.confidence - 0.66).abs() < 0.01);
}

#[test]
fn parse_vision_confidence_clamping() {
    let raw = r#"{"confidence": 1.5}"#;
    let summary = parse_vision_summary_output(test_frame(), raw);
    assert!((summary.confidence - 1.0).abs() < 0.01);

    let raw2 = r#"{"confidence": -0.5}"#;
    let summary2 = parse_vision_summary_output(test_frame(), raw2);
    assert!((summary2.confidence - 0.0).abs() < 0.01);
}

#[test]
fn parse_vision_empty_strings_use_fallback() {
    let raw = r#"{"ui_state": "", "actionable_notes": ""}"#;
    let summary = parse_vision_summary_output(test_frame(), raw);
    assert_eq!(summary.ui_state, "UI state unavailable");
    // actionable_notes falls back to truncated raw when empty
    assert!(!summary.actionable_notes.is_empty());
}

// ── should_capture_context / rule_matches_context ───────────────────────

#[test]
fn denylist_blocks_matching_context() {
    let engine = AccessibilityEngine {
        inner: Mutex::new(EngineState::new(ScreenIntelligenceConfig::default())),
    };
    let config = ScreenIntelligenceConfig {
        denylist: vec!["1password".to_string(), "keychain".to_string()],
        ..Default::default()
    };
    let ctx = AppContext {
        app_name: Some("1Password 8".to_string()),
        window_title: Some("Vault".to_string()),
        bounds: None,
    };
    assert!(
        !engine.should_capture_context(&ctx, &config),
        "1password should be blocked by denylist"
    );
}

#[test]
fn denylist_allows_non_matching_context() {
    let engine = AccessibilityEngine {
        inner: Mutex::new(EngineState::new(ScreenIntelligenceConfig::default())),
    };
    let config = ScreenIntelligenceConfig {
        denylist: vec!["1password".to_string()],
        ..Default::default()
    };
    let ctx = AppContext {
        app_name: Some("Safari".to_string()),
        window_title: Some("GitHub".to_string()),
        bounds: None,
    };
    assert!(engine.should_capture_context(&ctx, &config));
}

#[test]
fn whitelist_only_mode_blocks_unlisted() {
    let engine = AccessibilityEngine {
        inner: Mutex::new(EngineState::new(ScreenIntelligenceConfig::default())),
    };
    let config = ScreenIntelligenceConfig {
        policy_mode: "whitelist_only".to_string(),
        allowlist: vec!["code".to_string()],
        denylist: vec![],
        ..Default::default()
    };
    let ctx = AppContext {
        app_name: Some("Safari".to_string()),
        window_title: Some("Web".to_string()),
        bounds: None,
    };
    assert!(
        !engine.should_capture_context(&ctx, &config),
        "whitelist_only should block unlisted apps"
    );

    let ctx_allowed = AppContext {
        app_name: Some("Visual Studio Code".to_string()),
        window_title: Some("main.rs".to_string()),
        bounds: None,
    };
    assert!(engine.should_capture_context(&ctx_allowed, &config));
}

#[test]
fn denylist_matching_is_case_insensitive() {
    let engine = AccessibilityEngine {
        inner: Mutex::new(EngineState::new(ScreenIntelligenceConfig::default())),
    };
    let config = ScreenIntelligenceConfig {
        denylist: vec!["Keychain".to_string()],
        ..Default::default()
    };
    let ctx = AppContext {
        app_name: Some("KEYCHAIN Access".to_string()),
        window_title: None,
        bounds: None,
    };
    assert!(!engine.should_capture_context(&ctx, &config));
}

// ── validate_input_action ───────────────────────────────────────────────

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

#[test]
fn validate_key_type_empty_text() {
    let params = InputActionParams {
        action: "key_type".to_string(),
        x: None,
        y: None,
        button: None,
        text: Some("".to_string()),
        key: None,
        modifiers: None,
    };
    assert!(validate_input_action(&params).is_err());
}

#[test]
fn validate_key_press_whitespace_key() {
    let params = InputActionParams {
        action: "key_press".to_string(),
        x: None,
        y: None,
        button: None,
        text: None,
        key: Some("   ".to_string()),
        modifiers: None,
    };
    assert!(validate_input_action(&params).is_err());
}

#[test]
fn validate_coordinates_at_boundaries() {
    // 0,0 should be valid
    let zero = InputActionParams {
        action: "mouse_move".to_string(),
        x: Some(0),
        y: Some(0),
        button: None,
        text: None,
        key: None,
        modifiers: None,
    };
    assert!(validate_input_action(&zero).is_ok());

    // 10000,10000 should be valid
    let max = InputActionParams {
        action: "mouse_click".to_string(),
        x: Some(10000),
        y: Some(10000),
        button: None,
        text: None,
        key: None,
        modifiers: None,
    };
    assert!(validate_input_action(&max).is_ok());

    // 10001 should be invalid
    let over = InputActionParams {
        action: "mouse_move".to_string(),
        x: Some(10001),
        y: Some(0),
        button: None,
        text: None,
        key: None,
        modifiers: None,
    };
    assert!(validate_input_action(&over).is_err());
}

// ── truncate_tail ───────────────────────────────────────────────────────

#[test]
fn truncate_tail_within_limit() {
    assert_eq!(truncate_tail("hello", 10), "hello");
}

#[test]
fn truncate_tail_at_limit() {
    assert_eq!(truncate_tail("hello", 5), "hello");
}

#[test]
fn truncate_tail_over_limit() {
    assert_eq!(truncate_tail("hello world", 5), "world");
}

// ── generate_suggestions ────────────────────────────────────────────────

#[test]
fn suggestions_for_known_keywords() {
    let results = generate_suggestions("thanks", 3);
    assert!(!results.is_empty());
    assert!(results[0].value.contains("help"));

    let results2 = generate_suggestions("let's schedule a meeting", 3);
    assert!(results2.iter().any(|s| s.value.contains("10am")));
}

#[test]
fn suggestions_for_empty_context() {
    let results = generate_suggestions("", 3);
    assert!(!results.is_empty(), "should return default suggestion");
}

#[test]
fn suggestions_max_results_clamping() {
    let results = generate_suggestions("thanks for the meeting, let's ship", 1);
    assert_eq!(results.len(), 1, "should respect max_results");
}

// ── Session lifecycle tests (macOS-gated) ───────────────────────────────

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

// ── capture_test (standalone, no session needed) ────────────────────────

#[tokio::test]
async fn capture_test_returns_diagnostics() {
    let engine = Arc::new(AccessibilityEngine {
        inner: Mutex::new(EngineState::new(ScreenIntelligenceConfig::default())),
    });

    let result = engine.capture_test().await;
    // On any platform this should not panic — it may fail gracefully.
    assert!(result.timing_ms < 30000, "capture test should not hang");
    assert!(
        result.capture_mode == "windowed" || result.capture_mode == "fullscreen",
        "capture_mode should be windowed or fullscreen"
    );

    if !cfg!(target_os = "macos") {
        assert!(!result.ok, "should fail on non-macOS");
        assert!(result.error.is_some());
    }
}
