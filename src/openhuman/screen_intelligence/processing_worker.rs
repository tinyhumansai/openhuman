//! Vision processing worker — receives captured frames, runs OCR + LLM
//! analysis, and persists the synthesized document to unified memory.
//!
//! Pipeline per frame:
//!   1. Apple Vision OCR  (Swift, ~200ms) → raw text extraction
//!   2. Vision LLM        (Ollama, ~2-5s) → app/activity/focus/mood context
//!   3. Synthesis LLM     (Ollama, ~3-5s) → final informative document
//!   4. Persist to unified memory as markdown with YAML frontmatter

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use crate::openhuman::config::Config;
use crate::openhuman::local_ai;

use super::engine::AccessibilityEngine;
use super::helpers::{persist_vision_summary, push_ephemeral_vision_summary, truncate_tail};
use super::types::{CaptureFrame, VisionSummary};

/// Main processing loop. Receives frames from the capture worker channel.
pub(crate) async fn run(
    engine: Arc<AccessibilityEngine>,
    mut rx: tokio::sync::mpsc::UnboundedReceiver<CaptureFrame>,
) {
    tracing::debug!("[processing_worker] started");

    let mut processed_timestamps: HashSet<i64> = HashSet::new();

    while let Some(mut frame) = rx.recv().await {
        // Drain channel — keep only the latest frame.
        let mut skipped = 0u64;
        while let Ok(newer) = rx.try_recv() {
            skipped += 1;
            frame = newer;
        }
        if skipped > 0 {
            tracing::debug!(
                "[processing_worker] skipped {} stale frame(s), processing latest (ts={})",
                skipped,
                frame.captured_at_ms,
            );
            let mut state = engine.inner.lock().await;
            if let Some(session) = state.session.as_mut() {
                session.vision_queue_depth =
                    session.vision_queue_depth.saturating_sub(skipped as usize);
            }
        }

        // Skip already-processed frames.
        if processed_timestamps.contains(&frame.captured_at_ms) {
            tracing::debug!(
                "[processing_worker] frame ts={} already processed, skipping",
                frame.captured_at_ms,
            );
            let mut state = engine.inner.lock().await;
            if let Some(session) = state.session.as_mut() {
                session.vision_queue_depth = session.vision_queue_depth.saturating_sub(1);
            }
            continue;
        }

        tracing::debug!(
            "[processing_worker] processing frame (app={:?}, ts={}, reason={})",
            frame.app_name,
            frame.captured_at_ms,
            frame.reason
        );

        let keep_screenshots = engine.inner.lock().await.config.keep_screenshots;

        // Temp save for vision processing when keep_screenshots is off.
        let saved_path = if !keep_screenshots && frame.image_ref.is_some() {
            let workspace_dir = match Config::load_or_init().await {
                Ok(cfg) => cfg.workspace_dir.clone(),
                Err(_) => PathBuf::from("."),
            };
            match AccessibilityEngine::save_screenshot_to_disk(&workspace_dir, &frame) {
                Ok(path) => Some(path),
                Err(err) => {
                    tracing::debug!("[processing_worker] temp save failed: {err}");
                    None
                }
            }
        } else {
            None
        };

        {
            let mut state = engine.inner.lock().await;
            if let Some(session) = state.session.as_mut() {
                session.vision_state = "processing".to_string();
            } else {
                tracing::debug!("[processing_worker] no session, exiting");
                break;
            }
        }

        let capture_ts = frame.captured_at_ms;
        let result = analyze_frame(&engine, frame).await;

        // Mark processed.
        processed_timestamps.insert(capture_ts);
        if processed_timestamps.len() > 500 {
            let oldest = *processed_timestamps.iter().min().unwrap();
            processed_timestamps.remove(&oldest);
        }

        // Clean up temp screenshot.
        if !keep_screenshots {
            if let Some(path) = saved_path {
                let _ = std::fs::remove_file(&path);
            }
        }

        // Update session state and persist.
        let mut summary_to_persist: Option<VisionSummary> = None;
        {
            let mut state = engine.inner.lock().await;
            let Some(session) = state.session.as_mut() else {
                break;
            };
            session.vision_queue_depth = session.vision_queue_depth.saturating_sub(1);
            match result {
                Ok(summary) => {
                    tracing::debug!(
                        "[processing_worker] analysis complete (id={} confidence={:.2})",
                        summary.id,
                        summary.confidence
                    );
                    push_ephemeral_vision_summary(&mut session.vision_summaries, summary.clone());
                    session.last_vision_at_ms = Some(summary.captured_at_ms);
                    session.last_vision_summary = Some(summary.key_text.clone());
                    session.vision_state = "ready".to_string();
                    summary_to_persist = Some(summary);
                }
                Err(err) => {
                    tracing::debug!("[processing_worker] analysis failed: {err}");
                    session.vision_state = "error".to_string();
                    state.last_error = Some(err);
                }
            }
        }

        if let Some(summary) = summary_to_persist {
            match persist_vision_summary(summary).await {
                Ok(persisted) => {
                    let mut state = engine.inner.lock().await;
                    if let Some(session) = state.session.as_mut() {
                        session.vision_persist_count =
                            session.vision_persist_count.saturating_add(1);
                        session.last_vision_persisted_key = Some(persisted.key.clone());
                        session.last_vision_persist_error = None;
                    }
                }
                Err(err) => {
                    tracing::debug!("[processing_worker] persistence failed: {err}");
                    let mut state = engine.inner.lock().await;
                    if let Some(session) = state.session.as_mut() {
                        session.vision_state = "error".to_string();
                        session.last_vision_persist_error = Some(err.clone());
                    }
                    state.last_error = Some(format!("vision_summary_persist_failed: {err}"));
                }
            }
        }
    }

    tracing::debug!("[processing_worker] exiting");
}

// ── Analysis pipeline ───────────────────────────────────────────────────

/// Run the full 3-pass analysis pipeline on a captured frame.
async fn analyze_frame(
    engine: &AccessibilityEngine,
    frame: CaptureFrame,
) -> Result<VisionSummary, String> {
    let image_ref = frame
        .image_ref
        .clone()
        .ok_or_else(|| "frame has no image payload".to_string())?;

    // ── Pass 1: OCR via Apple Vision ────────────────────────────────
    tracing::debug!("[processing_worker] pass 1/3: Apple Vision OCR");
    let ocr_text = run_apple_vision_ocr(&image_ref)?;
    tracing::debug!(
        "[processing_worker] OCR extracted {} chars",
        ocr_text.len()
    );

    // ── Pass 2: Vision LLM for context ──────────────────────────────
    let compressed = super::image_processing::compress_screenshot(&image_ref, None, None)
        .map_err(|e| format!("image compression failed: {e}"))?;
    let vision_image_ref = compressed.data_uri;

    let config = Config::load_or_init()
        .await
        .map_err(|e| format!("failed to load config: {e}"))?;
    if !config.local_ai.enabled {
        return Err(
            "screen intelligence vision requires local_ai.enabled=true in config".to_string(),
        );
    }
    let provider = config.local_ai.provider.trim().to_ascii_lowercase();
    if provider != "ollama" {
        return Err(format!(
            "screen intelligence vision requires provider 'ollama' (found '{provider}')",
        ));
    }

    tracing::debug!(
        "[processing_worker] pass 2/3: vision LLM (model={})",
        config.local_ai.vision_model_id,
    );
    let service = local_ai::global(&config);
    let vision_prompt = r#"Describe this screenshot briefly. Answer each on its own line:

APP: Name the application and the specific page/view/tab shown.
DOING: What is the user actively doing? (e.g. writing code, reading email, browsing, chatting)
FOCUS: What's the main content area about? (e.g. a PR review for auth refactor, a Slack thread about deployment)
MOOD: Is anything urgent, broken, or notable? (errors, notifications, warnings — or "nothing notable")

One line per answer. No text extraction. Be specific and concise."#;
    let vision_context = service
        .vision_prompt(&config, vision_prompt, &[vision_image_ref], Some(150))
        .await?
        .trim()
        .to_string();

    // ── Pass 3: Synthesis LLM — final document ──────────────────────
    let app_label = frame.app_name.as_deref().unwrap_or("Unknown");
    let window_label = frame.window_title.as_deref().unwrap_or("");
    let ocr_truncated = truncate_tail(&ocr_text, 4000);

    tracing::debug!(
        "[processing_worker] pass 3/3: synthesis LLM (ocr={} chars, vision={} chars)",
        ocr_truncated.len(),
        vision_context.len(),
    );

    let synthesis_prompt = format!(
        r#"You are summarizing what a user is doing on their computer right now.

Application: {app_label}
Window: {window_label}

Visual context from the screenshot:
{vision_context}

Extracted text from screen (OCR):
{ocr_truncated}

Write a clear, informative summary in plain text. Include:
- What application and specific view/page the user has open
- What they are actively doing (coding, reading, chatting, browsing, etc.)
- Key content visible — summarize what's on screen (don't just list raw OCR, synthesize it)
- Any notable items: errors, notifications, deadlines, action items
- Brief context that would help someone understand this moment later

Be specific and informative. Write in present tense. ~300-500 words."#
    );

    let synthesis = service
        .prompt(&config, &synthesis_prompt, Some(700), true)
        .await
        .unwrap_or_else(|e| {
            tracing::debug!("[processing_worker] synthesis failed, using fallback: {e}");
            format!("{}\n\n{}", vision_context, ocr_truncated)
        });

    tracing::debug!(
        "[processing_worker] synthesis complete ({} chars)",
        synthesis.len(),
    );

    Ok(VisionSummary {
        id: format!(
            "vision-{}-{}",
            frame.captured_at_ms,
            uuid::Uuid::new_v4()
        ),
        captured_at_ms: frame.captured_at_ms,
        app_name: frame.app_name,
        window_title: frame.window_title,
        ui_state: truncate_tail(&vision_context, 500),
        key_text: truncate_tail(&synthesis, 4000),
        actionable_notes: String::new(),
        confidence: 0.9,
    })
}

// ── Apple Vision OCR ────────────────────────────────────────────────────

/// Run Apple Vision framework OCR on a base64-encoded image.
fn run_apple_vision_ocr(image_ref: &str) -> Result<String, String> {
    use base64::{engine::general_purpose::STANDARD as B64, Engine};

    let b64_payload = if let Some(pos) = image_ref.find(";base64,") {
        &image_ref[pos + 8..]
    } else {
        image_ref
    };

    let raw_bytes = B64
        .decode(b64_payload)
        .map_err(|e| format!("base64 decode for OCR failed: {e}"))?;

    let tmp_path = std::env::temp_dir().join(format!(
        "openhuman_ocr_{}.png",
        uuid::Uuid::new_v4()
    ));
    std::fs::write(&tmp_path, &raw_bytes)
        .map_err(|e| format!("failed to write temp OCR image: {e}"))?;

    let swift_code = format!(
        r#"
import Vision
import AppKit

let url = URL(fileURLWithPath: "{path}")
guard let image = NSImage(contentsOf: url),
      let cgImage = image.cgImage(forProposedRect: nil, context: nil, hints: nil) else {{
    fputs("ERROR: failed to load image\n", stderr)
    exit(1)
}}

let request = VNRecognizeTextRequest()
request.recognitionLevel = .accurate
request.usesLanguageCorrection = true

let handler = VNImageRequestHandler(cgImage: cgImage, options: [:])
try handler.perform([request])

guard let observations = request.results else {{
    exit(0)
}}

for obs in observations {{
    if let candidate = obs.topCandidates(1).first {{
        print(candidate.string)
    }}
}}
"#,
        path = tmp_path.display()
    );

    let output = std::process::Command::new("swift")
        .arg("-e")
        .arg(&swift_code)
        .output()
        .map_err(|e| format!("swift OCR failed to start: {e}"))?;

    let _ = std::fs::remove_file(&tmp_path);

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Apple Vision OCR failed: {}", stderr.trim()));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}
