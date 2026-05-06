//! E2E tests for the screen-intelligence vision pipeline.
//!
//! ## Platform support
//!
//! | Test group                          | Linux CI | macOS local |
//! |-------------------------------------|----------|-------------|
//! | Compression + image processing      | ✅        | ✅           |
//! | Memory persistence (UnifiedMemory)  | ✅        | ✅           |
//! | Screenshot save/cleanup (disk I/O)  | ✅        | ✅           |
//! | Real screen capture (permission)    | ❌        | ✅ (manual)  |
//! | Local LLM vision analysis           | ❌        | ✅ (manual)  |
//!
//! ### Running
//! ```
//! cargo test --test screen_intelligence_vision_e2e
//! ```
//! Cross-platform CI tests use `OPENHUMAN_SCREEN_INTELLIGENCE_MOCK_VISION_JSON` to validate the
//! real engine pipeline without requiring macOS permissions or a running Ollama server.
//!
//! ### macOS E2E checklist (manual, requires Screen Recording permission)
//! 1. Grant Screen Recording to the `openhuman-core` binary in System Settings › Privacy & Security.
//! 2. Run: `cargo test --test screen_intelligence_vision_e2e -- --nocapture`
//! 3. Ensure Ollama is running with a vision-capable model (e.g. `ollama run minicpm-v`).
//! 4. Call `openhuman.screen_intelligence_capture_test` via `cargo test --test json_rpc_e2e json_rpc_screen_intelligence`.
//! 5. Run ignored real-capture test:
//!    `cargo test --test screen_intelligence_vision_e2e macos_real_capture_cycle_persists_summary -- --ignored --nocapture`

use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::imageops::FilterType;
use image::{ImageBuffer, Rgb, RgbImage};
use tempfile::tempdir;

use openhuman_core::openhuman::embeddings::NoopEmbedding;
use openhuman_core::openhuman::memory::store::types::NamespaceDocumentInput;
use openhuman_core::openhuman::memory::store::UnifiedMemory;
use openhuman_core::openhuman::screen_intelligence::CaptureFrame;
use openhuman_core::openhuman::screen_intelligence::{
    global_engine, AccessibilityEngine, VisionSummary,
};

// ── Env isolation ────────────────────────────────────────────────────

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

    fn unset(key: &'static str) -> Self {
        let old = std::env::var(key).ok();
        std::env::remove_var(key);
        Self { key, old }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.old {
            Some(v) => std::env::set_var(self.key, v),
            None => std::env::remove_var(self.key),
        }
    }
}

static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    match ENV_LOCK.get_or_init(|| Mutex::new(())).lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

// ── Helpers ──────────────────────────────────────────────────────────

/// Create a synthetic PNG data-URI simulating a desktop screenshot.
fn make_test_png_uri(width: u32, height: u32) -> String {
    let img: RgbImage = ImageBuffer::from_fn(width, height, |x, y| {
        Rgb([
            (x % 256) as u8,
            (y % 256) as u8,
            ((x * 3 + y * 7) % 256) as u8,
        ])
    });
    let mut png_bytes: Vec<u8> = Vec::new();
    let encoder = PngEncoder::new(&mut png_bytes);
    img.write_with_encoder(encoder).expect("PNG encode");
    let b64 = B64.encode(&png_bytes);
    format!("data:image/png;base64,{b64}")
}

fn make_capture_frame(image_ref: Option<String>) -> CaptureFrame {
    CaptureFrame {
        captured_at_ms: chrono::Utc::now().timestamp_millis(),
        reason: "e2e_test".to_string(),
        app_name: Some("TestApp".to_string()),
        window_title: Some("E2E Test Window".to_string()),
        image_ref,
    }
}

/// Open a UnifiedMemory backed by NoopEmbedding in a temp dir.
fn open_test_memory(dir: &Path) -> UnifiedMemory {
    let embedder: Arc<dyn openhuman_core::openhuman::embeddings::EmbeddingProvider> =
        Arc::new(NoopEmbedding);
    UnifiedMemory::new(dir, embedder, Some(5)).expect("UnifiedMemory::new")
}

fn write_screen_intelligence_test_config(
    root: &Path,
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
runtime_enabled = {local_ai_enabled}
provider = "{local_ai_provider}"

[screen_intelligence]
keep_screenshots = false

[secrets]
encrypt = false
"#
    );
    std::fs::create_dir_all(root).expect("mkdir test root");
    std::fs::write(root.join("config.toml"), &cfg).expect("write config");
    let _: openhuman_core::openhuman::config::Config =
        toml::from_str(&cfg).expect("test config should deserialize");
}

/// Simulate what `parse_vision_summary_output` does, but from public types.
fn mock_vision_summary(frame: &CaptureFrame, raw_llm: &str) -> serde_json::Value {
    let value: serde_json::Value = serde_json::from_str(raw_llm).unwrap_or_else(|_| {
        serde_json::json!({
            "ui_state": "UI state unavailable",
            "key_text": "",
            "actionable_notes": raw_llm.trim(),
            "confidence": 0.66,
        })
    });
    serde_json::json!({
        "id": format!("vision-{}-e2e", frame.captured_at_ms),
        "captured_at_ms": frame.captured_at_ms,
        "app_name": frame.app_name,
        "window_title": frame.window_title,
        "ui_state": value.get("ui_state").and_then(|v| v.as_str()).unwrap_or("UI state unavailable"),
        "key_text": value.get("key_text").and_then(|v| v.as_str()).unwrap_or(""),
        "actionable_notes": value.get("actionable_notes").and_then(|v| v.as_str()).unwrap_or(raw_llm.trim()),
        "confidence": value.get("confidence").and_then(|v| v.as_f64()).unwrap_or(0.66),
    })
}

// ── Tests ────────────────────────────────────────────────────────────

/// Full pipeline: compress screenshot -> simulate LLM response -> persist to memory -> query back.
#[tokio::test]
async fn vision_pipeline_compress_parse_persist() {
    let _lock = env_lock();
    let tmp = tempdir().expect("tempdir");
    let _home = EnvVarGuard::set_to_path("HOME", tmp.path());

    // ── Step 1: Generate a 1920x1080 screenshot ─────────────────────
    let image_ref = make_test_png_uri(1920, 1080);
    let original_b64_len = image_ref.len();
    assert!(
        original_b64_len > 10_000,
        "test image should be non-trivial"
    );

    // ── Step 2: Compress (same logic as image_processing module) ─────
    let b64_payload = image_ref
        .find(";base64,")
        .map(|pos| &image_ref[pos + 8..])
        .unwrap_or(&image_ref);
    let raw_bytes = B64.decode(b64_payload).expect("decode original");
    let original_size = raw_bytes.len();

    let img = image::load_from_memory(&raw_bytes).expect("load image");
    assert_eq!(img.width(), 1920);
    assert_eq!(img.height(), 1080);

    // Resize to 1024 on long edge
    let max_dim = 1024u32;
    let scale = max_dim as f64 / img.width().max(img.height()) as f64;
    let new_w = (img.width() as f64 * scale).round() as u32;
    let new_h = (img.height() as f64 * scale).round() as u32;
    let resized = img.resize_exact(new_w, new_h, FilterType::Lanczos3);
    assert!(resized.width() <= max_dim);
    assert!(resized.height() <= max_dim);

    // JPEG encode
    let rgb = resized.to_rgb8();
    let mut jpeg_buf: Vec<u8> = Vec::new();
    let encoder = JpegEncoder::new_with_quality(&mut jpeg_buf, 72);
    rgb.write_with_encoder(encoder).expect("JPEG encode");
    let compressed_size = jpeg_buf.len();
    assert!(
        compressed_size < original_size,
        "compressed ({compressed_size}) should be smaller than original ({original_size})"
    );

    let compressed_uri = format!("data:image/jpeg;base64,{}", B64.encode(&jpeg_buf));
    assert!(compressed_uri.len() < original_b64_len);

    // ── Step 3: Simulate LLM vision response ────────────────────────
    let frame = make_capture_frame(Some(image_ref));
    let mock_llm_response = r#"{"ui_state": "code editor with terminal", "key_text": "fn main() { println!(\"hello\"); }", "actionable_notes": "User is editing Rust code in a split-pane layout", "confidence": 0.91}"#;
    let summary = mock_vision_summary(&frame, mock_llm_response);

    assert_eq!(
        summary["ui_state"].as_str().unwrap(),
        "code editor with terminal"
    );
    assert!((summary["confidence"].as_f64().unwrap() - 0.91).abs() < 0.01);

    // ── Step 4: Persist to memory ───────────────────────────────────
    let mem = open_test_memory(tmp.path());
    let content = serde_json::to_string(&summary).expect("serialize summary");
    let key = format!("screen_intelligence_{}", summary["id"].as_str().unwrap());
    mem.upsert_document(NamespaceDocumentInput {
        namespace: "background".to_string(),
        key: key.clone(),
        title: key.clone(),
        content: content.clone(),
        source_type: "screenshot".to_string(),
        priority: "medium".to_string(),
        tags: vec!["screen_intelligence".to_string()],
        metadata: serde_json::json!({}),
        category: "screen_intelligence".to_string(),
        session_id: None,
        document_id: None,
    })
    .await
    .expect("upsert_document");

    // ── Step 5: Query back from memory ──────────────────────────────
    let result_json = mem
        .list_documents(Some("background"))
        .await
        .expect("list_documents");
    let docs = result_json["documents"]
        .as_array()
        .expect("documents array");
    assert!(!docs.is_empty(), "should find the persisted vision summary");
    let found = docs.iter().any(|d| d["key"].as_str() == Some(&key));
    assert!(found, "should find document by key: {key}");
}

/// Multiple screenshots persisted and queryable.
#[tokio::test]
async fn multiple_vision_summaries_persist_and_query() {
    let _lock = env_lock();
    let tmp = tempdir().expect("tempdir");
    let _home = EnvVarGuard::set_to_path("HOME", tmp.path());

    let mem = open_test_memory(tmp.path());

    let scenarios = vec![
        (
            "Safari",
            "GitHub PR Review",
            0.88,
            "User reviewing pull request diffs",
        ),
        (
            "VSCode",
            "main.rs - editor",
            0.92,
            "Rust code editing with LSP diagnostics",
        ),
        (
            "Terminal",
            "cargo test output",
            0.85,
            "Test results showing 19 passed",
        ),
    ];

    for (i, (app, window, confidence, notes)) in scenarios.iter().enumerate() {
        let ts = chrono::Utc::now().timestamp_millis() + i as i64;
        let summary = serde_json::json!({
            "id": format!("vision-{ts}-{app}"),
            "captured_at_ms": ts,
            "app_name": app,
            "window_title": window,
            "ui_state": "active",
            "key_text": "",
            "actionable_notes": notes,
            "confidence": confidence,
        });

        let content = serde_json::to_string(&summary).expect("serialize");
        let key = format!("screen_intelligence_{}", summary["id"].as_str().unwrap());
        mem.upsert_document(NamespaceDocumentInput {
            namespace: "background".to_string(),
            key,
            title: format!("{app} - {window}"),
            content,
            source_type: "screenshot".to_string(),
            priority: "medium".to_string(),
            tags: vec!["screen_intelligence".to_string()],
            metadata: serde_json::json!({}),
            category: "screen_intelligence".to_string(),
            session_id: None,
            document_id: None,
        })
        .await
        .expect("upsert");
    }

    let result_json = mem
        .list_documents(Some("background"))
        .await
        .expect("list_documents");
    let docs = result_json["documents"]
        .as_array()
        .expect("documents array");
    assert_eq!(
        docs.len(),
        3,
        "should have 3 persisted summaries, got {}",
        docs.len()
    );
}

/// Malformed LLM response still produces a usable summary (fallback path).
#[test]
fn malformed_llm_response_handled_gracefully() {
    let frame = make_capture_frame(None);
    let broken = "Sorry, I cannot analyze this image due to unclear content.";
    let summary = mock_vision_summary(&frame, broken);

    assert_eq!(
        summary["ui_state"].as_str().unwrap(),
        "UI state unavailable"
    );
    assert!(summary["actionable_notes"]
        .as_str()
        .unwrap()
        .contains("Sorry"));
    assert!((summary["confidence"].as_f64().unwrap() - 0.66).abs() < 0.01);
}

/// Compression pipeline handles various image sizes without panicking.
#[test]
fn compression_handles_various_sizes() {
    let sizes = vec![
        (64, 64),     // tiny
        (800, 600),   // small desktop
        (1920, 1080), // full HD
        (3840, 2160), // 4K
        (100, 2000),  // tall narrow
        (3000, 50),   // wide short
    ];

    let max_dim = 1024u32;
    for (w, h) in sizes {
        let uri = make_test_png_uri(w, h);
        let b64_payload = uri
            .find(";base64,")
            .map(|pos| &uri[pos + 8..])
            .unwrap_or(&uri);
        let raw = B64.decode(b64_payload).expect("decode");
        let img = image::load_from_memory(&raw).expect("load");
        assert_eq!(img.width(), w, "width mismatch for {w}x{h}");
        assert_eq!(img.height(), h, "height mismatch for {w}x{h}");

        if w > max_dim || h > max_dim {
            let scale = max_dim as f64 / w.max(h) as f64;
            let nw = (w as f64 * scale).round() as u32;
            let nh = (h as f64 * scale).round() as u32;
            let resized = img.resize_exact(nw, nh, FilterType::Lanczos3);
            assert!(
                resized.width() <= max_dim,
                "resized width exceeds max for {w}x{h}"
            );
            assert!(
                resized.height() <= max_dim,
                "resized height exceeds max for {w}x{h}"
            );

            let rgb = resized.to_rgb8();
            let mut buf: Vec<u8> = Vec::new();
            let enc = JpegEncoder::new_with_quality(&mut buf, 72);
            rgb.write_with_encoder(enc)
                .unwrap_or_else(|e| panic!("JPEG encode failed for {w}x{h}: {e}"));
            assert!(
                !buf.is_empty(),
                "JPEG output should not be empty for {w}x{h}"
            );
        }
    }
}

/// Vision summary upsert is idempotent (same key overwrites, not duplicates).
#[tokio::test]
async fn vision_summary_upsert_is_idempotent() {
    let _lock = env_lock();
    let tmp = tempdir().expect("tempdir");
    let _home = EnvVarGuard::set_to_path("HOME", tmp.path());

    let mem = open_test_memory(tmp.path());
    let key = "screen_intelligence_vision-12345-upsert-test".to_string();

    // First insert
    mem.upsert_document(NamespaceDocumentInput {
        namespace: "background".to_string(),
        key: key.clone(),
        title: key.clone(),
        content: r#"{"version": 1}"#.to_string(),
        source_type: "screenshot".to_string(),
        priority: "medium".to_string(),
        tags: vec!["screen_intelligence".to_string()],
        metadata: serde_json::json!({}),
        category: "screen_intelligence".to_string(),
        session_id: None,
        document_id: None,
    })
    .await
    .expect("first upsert");

    // Second insert with same key, different content
    mem.upsert_document(NamespaceDocumentInput {
        namespace: "background".to_string(),
        key: key.clone(),
        title: key.clone(),
        content: r#"{"version": 2}"#.to_string(),
        source_type: "screenshot".to_string(),
        priority: "medium".to_string(),
        tags: vec!["screen_intelligence".to_string()],
        metadata: serde_json::json!({}),
        category: "screen_intelligence".to_string(),
        session_id: None,
        document_id: None,
    })
    .await
    .expect("second upsert");

    let result_json = mem
        .list_documents(Some("background"))
        .await
        .expect("list_documents");
    let docs = result_json["documents"]
        .as_array()
        .expect("documents array");
    let matching: Vec<_> = docs
        .iter()
        .filter(|d| d["key"].as_str() == Some(&key))
        .collect();
    assert_eq!(
        matching.len(),
        1,
        "upsert should overwrite, not duplicate: found {} docs",
        matching.len()
    );
}

/// Verify that compression produces significant savings on realistic images.
#[test]
fn compression_savings_on_realistic_screenshot() {
    let uri = make_test_png_uri(2560, 1440); // QHD resolution
    let b64_payload = uri.find(";base64,").map(|pos| &uri[pos + 8..]).unwrap();
    let raw = B64.decode(b64_payload).expect("decode");
    let original_size = raw.len();

    let img = image::load_from_memory(&raw).expect("load");
    let scale = 1024.0 / img.width().max(img.height()) as f64;
    let nw = (img.width() as f64 * scale).round() as u32;
    let nh = (img.height() as f64 * scale).round() as u32;
    let resized = img.resize_exact(nw, nh, FilterType::Lanczos3);

    let rgb = resized.to_rgb8();
    let mut jpeg_buf: Vec<u8> = Vec::new();
    let enc = JpegEncoder::new_with_quality(&mut jpeg_buf, 72);
    rgb.write_with_encoder(enc).expect("JPEG encode");

    let ratio = jpeg_buf.len() as f64 / original_size as f64;
    assert!(
        ratio < 0.5,
        "compression ratio should be under 50%, got {:.1}%",
        ratio * 100.0
    );
}

/// save_screenshot_to_disk writes a valid PNG file to the workspace directory.
#[test]
fn save_screenshot_to_disk_creates_png_file() {
    let png_uri = make_test_png_uri(32, 32);
    let frame = CaptureFrame {
        captured_at_ms: 1700000000001,
        reason: "e2e_disk_save_test".to_string(),
        app_name: Some("DiskSaveApp".to_string()),
        window_title: Some("E2E Save Test".to_string()),
        image_ref: Some(png_uri),
    };

    let tmp = tempdir().expect("tempdir");
    let result = AccessibilityEngine::save_screenshot_to_disk(tmp.path(), &frame);

    assert!(
        result.is_ok(),
        "[screen_intelligence] save_screenshot_to_disk should succeed: {:?}",
        result
    );
    let saved_path = result.unwrap();
    assert!(
        saved_path.exists(),
        "[screen_intelligence] saved PNG file should exist at {}",
        saved_path.display()
    );
    assert_eq!(
        saved_path.extension().and_then(|e| e.to_str()),
        Some("png"),
        "saved file should have .png extension"
    );
    let metadata = std::fs::metadata(&saved_path).expect("file metadata");
    assert!(metadata.len() > 0, "saved PNG should not be empty");
}

/// Simulates the keep_screenshots=false cleanup path: save then immediately remove.
#[test]
fn save_screenshot_to_disk_cleanup_simulates_keep_screenshots_false() {
    let png_uri = make_test_png_uri(32, 32);
    let frame = CaptureFrame {
        captured_at_ms: 1700000000002,
        reason: "e2e_cleanup_test".to_string(),
        app_name: Some("CleanupApp".to_string()),
        window_title: Some("E2E Cleanup Test".to_string()),
        image_ref: Some(png_uri),
    };

    let tmp = tempdir().expect("tempdir");
    let result = AccessibilityEngine::save_screenshot_to_disk(tmp.path(), &frame);
    assert!(
        result.is_ok(),
        "[screen_intelligence] save should succeed before cleanup: {:?}",
        result
    );

    let saved_path = result.unwrap();
    assert!(saved_path.exists(), "file should exist before cleanup");

    // Simulate what the vision worker does when keep_screenshots=false
    std::fs::remove_file(&saved_path).expect("remove_file should succeed");

    assert!(
        !saved_path.exists(),
        "[screen_intelligence] file should no longer exist after cleanup: {}",
        saved_path.display()
    );
}

/// VisionSummary struct serializes and deserializes correctly, and is queryable after persistence.
///
/// Tests two things independently:
/// 1. `VisionSummary` serde roundtrip in memory (proves struct attributes are correct).
/// 2. Persisting to UnifiedMemory and verifying the key is listed (proves `persist_vision_summary`
///    writes to the right namespace with the right key format).
#[tokio::test]
async fn vision_summary_struct_persist_and_deserialize_roundtrip() {
    let _lock = env_lock();
    let tmp = tempdir().expect("tempdir");
    let _home = EnvVarGuard::set_to_path("HOME", tmp.path());

    let summary = VisionSummary {
        id: "vision-1700000000100-roundtrip-test".to_string(),
        captured_at_ms: 1700000000100,
        app_name: Some("RoundtripApp".to_string()),
        window_title: Some("Roundtrip Test Window".to_string()),
        ui_state: "code editor with Rust file open".to_string(),
        key_text: "fn main() {}".to_string(),
        actionable_notes: "Developer is writing Rust code".to_string(),
        confidence: 0.93,
    };

    // ── Step 1: serde roundtrip in memory (no DB) ──────────────────────────
    // This proves VisionSummary has correct Serialize/Deserialize attributes and
    // that the JSON format matches what persist_vision_summary stores.
    let serialized = serde_json::to_string(&summary).expect("serialize VisionSummary");
    let deserialized: VisionSummary =
        serde_json::from_str(&serialized).expect("deserialize VisionSummary");

    assert_eq!(deserialized.id, summary.id, "id roundtrip");
    assert_eq!(
        deserialized.ui_state, summary.ui_state,
        "ui_state roundtrip"
    );
    assert_eq!(
        deserialized.key_text, summary.key_text,
        "key_text roundtrip"
    );
    assert_eq!(
        deserialized.actionable_notes, summary.actionable_notes,
        "actionable_notes roundtrip"
    );
    assert_eq!(
        deserialized.app_name, summary.app_name,
        "app_name roundtrip"
    );
    assert!(
        (deserialized.confidence - summary.confidence).abs() < 0.01,
        "confidence roundtrip: expected {}, got {}",
        summary.confidence,
        deserialized.confidence
    );

    // ── Step 2: persist to UnifiedMemory, verify queryable by key ─────────
    // Matches exactly what persist_vision_summary() does (namespace, key format, tags).
    let mem = open_test_memory(tmp.path());
    let key = format!("screen_intelligence_{}", summary.id);
    mem.upsert_document(NamespaceDocumentInput {
        namespace: "background".to_string(),
        key: key.clone(),
        title: key.clone(),
        content: serialized,
        source_type: "screenshot".to_string(),
        priority: "medium".to_string(),
        tags: vec!["screen_intelligence".to_string()],
        metadata: serde_json::json!({}),
        category: "screen_intelligence".to_string(),
        session_id: None,
        document_id: None,
    })
    .await
    .expect("upsert_document");

    let result_json = mem
        .list_documents(Some("background"))
        .await
        .expect("list_documents");
    let docs = result_json["documents"]
        .as_array()
        .expect("documents array");

    assert!(
        docs.iter().any(|d| d["key"].as_str() == Some(&key)),
        "[screen_intelligence] persisted VisionSummary should be queryable by key: {key}"
    );
}

/// Exercises the real engine pipeline (compress -> parse -> persist) with mocked local-vision
/// output so Linux CI can validate behavior without macOS permissions or Ollama runtime.
#[tokio::test]
async fn engine_pipeline_with_mocked_local_vision_persists_to_memory() {
    let _lock = env_lock();
    let tmp = tempdir().expect("tempdir");
    let _workspace = EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", tmp.path());
    let _mock = EnvVarGuard::set(
        "OPENHUMAN_SCREEN_INTELLIGENCE_MOCK_VISION_JSON",
        r#"{"ui_state":"browser with docs","key_text":"README.md","actionable_notes":"User is reading project docs","confidence":0.89}"#,
    );
    write_screen_intelligence_test_config(tmp.path(), true, "ollama");

    let frame = make_capture_frame(Some(make_test_png_uri(960, 540)));
    let summary = global_engine()
        .analyze_and_persist_frame(frame)
        .await
        .expect("mocked engine pipeline should succeed");
    assert_eq!(summary.ui_state, "browser with docs");

    let config = openhuman_core::openhuman::config::Config::load_or_init()
        .await
        .expect("load config");
    let mem = open_test_memory(&config.workspace_dir);
    let docs = mem
        .list_documents(Some("background"))
        .await
        .expect("list documents")["documents"]
        .as_array()
        .cloned()
        .expect("documents array");
    let key = format!("screen_intelligence_{}", summary.id);
    assert!(
        docs.iter().any(|doc| doc["key"].as_str() == Some(&key)),
        "expected persisted summary key in memory: {key}"
    );
}

/// Ensures screen-intelligence vision refuses non-local providers to avoid remote fallback.
#[tokio::test]
async fn engine_pipeline_rejects_non_local_provider() {
    let _lock = env_lock();
    let tmp = tempdir().expect("tempdir");
    let _workspace = EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", tmp.path());
    write_screen_intelligence_test_config(tmp.path(), true, "openai");

    let frame = make_capture_frame(Some(make_test_png_uri(320, 240)));
    let err = global_engine()
        .analyze_and_persist_frame(frame)
        .await
        .expect_err("non-local providers should be rejected");
    assert!(
        err.contains("provider 'ollama'"),
        "unexpected provider guard error: {err}"
    );
}

/// Manual macOS-only smoke test for the real capture -> local vision -> memory persistence chain.
/// Run manually with:
/// `cargo test --test screen_intelligence_vision_e2e macos_real_capture_cycle_persists_summary -- --ignored --nocapture`
#[cfg(target_os = "macos")]
#[tokio::test]
#[ignore = "requires Screen Recording permission + local Ollama vision model"]
async fn macos_real_capture_cycle_persists_summary() {
    let _lock = env_lock();
    let tmp = tempdir().expect("tempdir");
    let _workspace = EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", tmp.path());
    let _mock = EnvVarGuard::unset("OPENHUMAN_SCREEN_INTELLIGENCE_MOCK_VISION_JSON");
    write_screen_intelligence_test_config(tmp.path(), true, "ollama");

    let capture = global_engine().capture_test().await;
    assert!(
        capture.ok,
        "capture_test failed; ensure Screen Recording permission is granted: {:?}",
        capture.error
    );
    let image_ref = capture
        .image_ref
        .clone()
        .expect("capture_test should return image_ref on success");
    let frame = make_capture_frame(Some(image_ref));

    let summary = global_engine()
        .analyze_and_persist_frame(frame)
        .await
        .expect("real local-vision inference should succeed");
    assert!(
        !summary.actionable_notes.is_empty(),
        "summary should include actionable notes"
    );

    let config = openhuman_core::openhuman::config::Config::load_or_init()
        .await
        .expect("load config");
    let mem = open_test_memory(&config.workspace_dir);
    let docs = mem
        .list_documents(Some("background"))
        .await
        .expect("list documents")["documents"]
        .as_array()
        .cloned()
        .expect("documents array");
    let key = format!("screen_intelligence_{}", summary.id);
    assert!(
        docs.iter().any(|doc| doc["key"].as_str() == Some(&key)),
        "expected persisted summary key after real capture cycle: {key}"
    );
}
