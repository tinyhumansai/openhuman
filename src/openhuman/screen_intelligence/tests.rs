use std::path::Path;
use std::sync::{Arc, OnceLock};
use tokio::sync::Mutex;
use tokio::time::{self, Duration};

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use image::codecs::png::PngEncoder;
use image::{ImageBuffer, Rgb, RgbImage};
use tempfile::tempdir;

use super::helpers::{
    generate_suggestions, parse_vision_summary_output, truncate_tail, validate_input_action,
};
use super::state::{AccessibilityEngine, EngineState};
use super::types::{CaptureFrame, InputActionParams, StartSessionParams};
use crate::openhuman::accessibility::{parse_foreground_output, AppContext};
use crate::openhuman::config::{Config, ScreenIntelligenceConfig};
use crate::openhuman::memory::embeddings::NoopEmbedding;
use crate::openhuman::memory::store::UnifiedMemory;

struct EnvVarGuard {
    key: &'static str,
    old: Option<String>,
}

impl EnvVarGuard {
    fn set_to_path(key: &'static str, path: &Path) -> Self {
        let old = std::env::var(key).ok();
        std::env::set_var(key, path.as_os_str());
        Self { key, old }
    }

    fn set(key: &'static str, value: &str) -> Self {
        let old = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, old }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.old {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

static SCREEN_INTELLIGENCE_ENV_LOCK: OnceLock<std::sync::Mutex<()>> = OnceLock::new();

fn screen_intelligence_env_lock() -> std::sync::MutexGuard<'static, ()> {
    match SCREEN_INTELLIGENCE_ENV_LOCK
        .get_or_init(|| std::sync::Mutex::new(()))
        .lock()
    {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn write_screen_intelligence_test_config(
    workspace_root: &Path,
    local_ai_enabled: bool,
    local_ai_provider: &str,
) {
    let cfg = format!(
        r#"default_temperature = 0.7

[memory]
backend = "sqlite"
auto_save = true
embedding_provider = "none"
embedding_model = "none"
embedding_dimensions = 0

[local_ai]
enabled = {local_ai_enabled}
provider = "{local_ai_provider}"

[secrets]
encrypt = false
"#
    );
    std::fs::create_dir_all(workspace_root).expect("mkdir test workspace root");
    let config_path = workspace_root.join("config.toml");
    std::fs::write(&config_path, &cfg).expect("write test config");
    let _: Config = toml::from_str(&cfg).expect("test config should deserialize");
}

fn make_test_png_uri(width: u32, height: u32) -> String {
    let img: RgbImage = ImageBuffer::from_fn(width, height, |x, y| {
        Rgb([(x % 255) as u8, (y % 255) as u8, ((x + y) % 255) as u8])
    });
    let mut png_bytes: Vec<u8> = Vec::new();
    img.write_with_encoder(PngEncoder::new(&mut png_bytes))
        .expect("PNG encode");
    format!("data:image/png;base64,{}", B64.encode(&png_bytes))
}

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
    // Plain text mode: first line = ui_state
    let raw = "this is not json at all";
    let summary = parse_vision_summary_output(test_frame(), raw);
    assert_eq!(summary.ui_state, "this is not json at all");
    assert!((summary.confidence - 0.8).abs() < 0.01);
}

#[test]
fn parse_vision_missing_fields() {
    let raw = r#"{"ui_state": "active"}"#;
    let summary = parse_vision_summary_output(test_frame(), raw);
    assert_eq!(summary.ui_state, "active");
    assert_eq!(summary.key_text, "");
    // Default confidence is now 0.8 (consistent across JSON and plain-text branches).
    assert!((summary.confidence - 0.8).abs() < 0.01);
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
    // JSON with empty strings — JSON path still works, empty fields stay empty
    let raw = r#"{"ui_state": "", "actionable_notes": ""}"#;
    let summary = parse_vision_summary_output(test_frame(), raw);
    assert_eq!(summary.ui_state, "");
    assert_eq!(summary.actionable_notes, "");
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
        window_id: None,
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
        window_id: None,
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
        window_id: None,
    };
    assert!(
        !engine.should_capture_context(&ctx, &config),
        "whitelist_only should block unlisted apps"
    );

    let ctx_allowed = AppContext {
        app_name: Some("Visual Studio Code".to_string()),
        window_title: Some("main.rs".to_string()),
        bounds: None,
        window_id: None,
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
        window_id: None,
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

    let engine = Arc::new(AccessibilityEngine {
        inner: Mutex::new(EngineState::new(ScreenIntelligenceConfig {
            baseline_fps: 6.0,
            session_ttl_secs: 60,
            ..Default::default()
        })),
    });

    let started = engine
        .start_session(StartSessionParams {
            consent: true,
            ttl_secs: Some(60),
            screen_monitoring: Some(true),
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
        })
        .await;

    if started.is_err() {
        return;
    }

    time::sleep(Duration::from_millis(700)).await;

    let status = engine.status().await;
    // The capture worker requires a valid window_id (CGWindowID) to capture.
    // In some environments (CI, headless, or when the foreground app doesn't
    // expose a Quartz window) no frames will be captured — skip gracefully.
    if status.session.frames_in_memory == 0 {
        let _ = engine
            .stop_session(Some("test_skip_no_window_id".to_string()))
            .await;
        return;
    }
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

    // On macOS dev machines without Screen Recording permission
    // granted to the cargo-test binary, `capture_test` blocks for
    // ~30s waiting on the macOS permission ticker and then returns
    // with a large `timing_ms`. That is an environment artefact,
    // not a product bug — treat it as "skip strict assertions" so
    // local runs stop failing while CI (Linux, no screen API at
    // all) still exercises the non-macOS assertion below.
    if cfg!(target_os = "macos") && result.timing_ms >= 10_000 {
        eprintln!(
            "[capture_test] capture_test() took {}ms — likely running \
             without Screen Recording permission. Skipping strict \
             assertions (this path passes in CI).",
            result.timing_ms
        );
        return;
    }

    assert!(
        result.capture_mode == "windowed" || result.capture_mode == "fullscreen",
        "capture_mode should be windowed or fullscreen"
    );

    if !cfg!(target_os = "macos") {
        assert!(!result.ok, "should fail on non-macOS");
        assert_eq!(
            result.error.as_deref(),
            Some("screen capture is unsupported on this platform")
        );
    }
}

#[tokio::test]
async fn capture_now_without_session_is_rejected_without_hanging() {
    let engine = Arc::new(AccessibilityEngine {
        inner: Mutex::new(EngineState::new(ScreenIntelligenceConfig::default())),
    });

    let result = engine
        .capture_now()
        .await
        .expect("capture_now should not error");
    assert!(
        !result.accepted,
        "capture_now should be rejected without a session"
    );
    assert!(
        result.frame.is_none(),
        "capture_now should not produce a frame without a session"
    );
}

// ── save_screenshot_to_disk ─────────────────────────────────────────────

#[test]
fn save_screenshot_to_disk_writes_png_to_workspace() {
    use base64::{engine::general_purpose::STANDARD as B64, Engine};
    use image::codecs::png::PngEncoder;
    use image::{ImageBuffer, Rgb, RgbImage};
    use tempfile::tempdir;

    let tmp = tempdir().expect("tempdir");

    // Build a tiny 4x4 solid-colour PNG as data URI
    let img: RgbImage = ImageBuffer::from_fn(4, 4, |_, _| Rgb([100u8, 149u8, 237u8]));
    let mut png_bytes: Vec<u8> = Vec::new();
    img.write_with_encoder(PngEncoder::new(&mut png_bytes))
        .expect("PNG encode");
    let image_ref = format!("data:image/png;base64,{}", B64.encode(&png_bytes));

    let frame = CaptureFrame {
        captured_at_ms: 1700000000200,
        reason: "unit_test_save".to_string(),
        app_name: Some("UnitTestApp".to_string()),
        window_title: Some("Test Window".to_string()),
        image_ref: Some(image_ref),
    };

    let result = AccessibilityEngine::save_screenshot_to_disk(tmp.path(), &frame);
    assert!(
        result.is_ok(),
        "save_screenshot_to_disk should succeed: {:?}",
        result
    );

    let path = result.unwrap();
    assert!(
        path.exists(),
        "[screen_intelligence] saved PNG file should exist at {}",
        path.display()
    );
    assert_eq!(
        path.extension().and_then(|e| e.to_str()),
        Some("png"),
        "saved file should have .png extension"
    );
    let metadata = std::fs::metadata(&path).expect("file metadata");
    assert!(metadata.len() > 0, "saved PNG should not be empty");
    assert!(
        path.to_string_lossy().contains("1700000000200"),
        "filename should include capture timestamp"
    );
}

#[test]
fn save_screenshot_to_disk_rejects_frame_without_image_ref() {
    use tempfile::tempdir;

    let tmp = tempdir().expect("tempdir");

    let frame = CaptureFrame {
        captured_at_ms: 1700000000201,
        reason: "unit_test_no_image".to_string(),
        app_name: Some("TestApp".to_string()),
        window_title: None,
        image_ref: None, // no image payload
    };

    let result = AccessibilityEngine::save_screenshot_to_disk(tmp.path(), &frame);
    assert!(
        result.is_err(),
        "[screen_intelligence] save_screenshot_to_disk should return Err when frame has no image_ref"
    );
    let err = result.unwrap_err();
    assert!(!err.is_empty(), "error message should not be empty");
}

// ── deterministic vision pipeline (mocked local output) ────────────────────

#[tokio::test]
async fn analyze_and_persist_frame_writes_unified_memory_document() {
    let _env_lock = screen_intelligence_env_lock();
    let tmp = tempdir().expect("tempdir");
    let _workspace = EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", tmp.path());
    let _mock = EnvVarGuard::set(
        "OPENHUMAN_SCREEN_INTELLIGENCE_MOCK_VISION_JSON",
        r#"{"ui_state":"editor","key_text":"fn main() {}","actionable_notes":"Rust source is open","confidence":0.93}"#,
    );
    write_screen_intelligence_test_config(tmp.path(), true, "ollama");

    let engine = Arc::new(AccessibilityEngine {
        inner: Mutex::new(EngineState::new(ScreenIntelligenceConfig::default())),
    });
    let frame = CaptureFrame {
        captured_at_ms: 1700000000300,
        reason: "pipeline_test".to_string(),
        app_name: Some("PipelineApp".to_string()),
        window_title: Some("Main.rs".to_string()),
        image_ref: Some(make_test_png_uri(320, 200)),
    };
    let summary = engine
        .analyze_and_persist_frame(frame)
        .await
        .expect("analyze_and_persist_frame should succeed with mocked vision output");
    assert_eq!(summary.ui_state, "editor");
    assert_eq!(summary.actionable_notes, "Rust source is open");

    let config = Config::load_or_init().await.expect("load config");
    let mem = UnifiedMemory::new(
        &config.workspace_dir,
        Arc::new(NoopEmbedding),
        config.memory.sqlite_open_timeout_secs,
    )
    .expect("memory init");
    let list = mem
        .list_documents(Some("background"))
        .await
        .expect("list documents");
    let documents = list["documents"]
        .as_array()
        .expect("documents array should exist");
    let key = format!("screen_intelligence_{}", summary.id);
    assert!(
        documents
            .iter()
            .any(|doc| doc["key"].as_str() == Some(&key)),
        "expected persisted vision summary key in background namespace: {key}"
    );
}

#[tokio::test]
async fn analyze_and_persist_frame_rejects_non_local_provider() {
    let _env_lock = screen_intelligence_env_lock();
    let tmp = tempdir().expect("tempdir");
    let _workspace = EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", tmp.path());
    write_screen_intelligence_test_config(tmp.path(), true, "openai");

    let engine = Arc::new(AccessibilityEngine {
        inner: Mutex::new(EngineState::new(ScreenIntelligenceConfig::default())),
    });
    let frame = CaptureFrame {
        captured_at_ms: 1700000000301,
        reason: "provider_guard_test".to_string(),
        app_name: Some("PipelineApp".to_string()),
        window_title: Some("Guard".to_string()),
        image_ref: Some(make_test_png_uri(160, 120)),
    };

    let err = engine
        .analyze_and_persist_frame(frame)
        .await
        .expect_err("non-local providers should be rejected");
    assert!(
        err.contains("provider 'ollama'"),
        "unexpected error for non-local provider: {err}"
    );
}

#[tokio::test]
async fn analyze_and_persist_frame_rejects_disabled_local_ai() {
    let _env_lock = screen_intelligence_env_lock();
    let tmp = tempdir().expect("tempdir");
    let _workspace = EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", tmp.path());
    write_screen_intelligence_test_config(tmp.path(), false, "ollama");

    let engine = Arc::new(AccessibilityEngine {
        inner: Mutex::new(EngineState::new(ScreenIntelligenceConfig::default())),
    });
    let frame = CaptureFrame {
        captured_at_ms: 1700000000302,
        reason: "local_ai_disabled_test".to_string(),
        app_name: Some("PipelineApp".to_string()),
        window_title: Some("Guard".to_string()),
        image_ref: Some(make_test_png_uri(160, 120)),
    };

    let err = engine
        .analyze_and_persist_frame(frame)
        .await
        .expect_err("disabled local ai should be rejected");
    assert!(
        err.contains("local_ai.enabled=true"),
        "unexpected error when local ai is disabled: {err}"
    );
}
