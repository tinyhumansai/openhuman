//! E2E tests for the screen-intelligence vision pipeline.
//!
//! Validates the full flow: generate image -> compress/resize -> parse vision
//! output -> persist to memory, all against real local storage in a temp workspace.
//!
//! Run with: `cargo test --test screen_intelligence_vision_e2e`

use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::imageops::FilterType;
use image::{ImageBuffer, Rgb, RgbImage};
use tempfile::tempdir;

use openhuman_core::openhuman::memory::embeddings::NoopEmbedding;
use openhuman_core::openhuman::memory::store::types::NamespaceDocumentInput;
use openhuman_core::openhuman::memory::store::UnifiedMemory;
use openhuman_core::openhuman::screen_intelligence::CaptureFrame;

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
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .expect("env lock poisoned")
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
    let embedder: Arc<dyn openhuman_core::openhuman::memory::embeddings::EmbeddingProvider> =
        Arc::new(NoopEmbedding);
    UnifiedMemory::new(dir, embedder, Some(5)).expect("UnifiedMemory::new")
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
