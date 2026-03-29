use crate::openhuman::config::Config;
use crate::openhuman::memory::{self, NamespaceDocumentInput, UnifiedMemory};
use std::collections::VecDeque;
use std::sync::Arc;
use uuid::Uuid;

use super::limits::{MAX_CONTEXT_CHARS, MAX_EPHEMERAL_FRAMES, MAX_EPHEMERAL_VISION_SUMMARIES};
use super::types::{AutocompleteSuggestion, CaptureFrame, InputActionParams, VisionSummary};

pub(crate) fn validate_input_action(action: &InputActionParams) -> Result<(), String> {
    match action.action.as_str() {
        "mouse_move" | "mouse_click" | "mouse_drag" => {
            let x = action
                .x
                .ok_or_else(|| "x coordinate is required".to_string())?;
            let y = action
                .y
                .ok_or_else(|| "y coordinate is required".to_string())?;
            if !(0..=10000).contains(&x) || !(0..=10000).contains(&y) {
                return Err("coordinates must be between 0 and 10000".to_string());
            }
        }
        "key_type" => {
            let text = action
                .text
                .as_ref()
                .ok_or_else(|| "text is required for key_type".to_string())?;
            if text.is_empty() || text.len() > MAX_CONTEXT_CHARS {
                return Err("text length must be between 1 and 256".to_string());
            }
        }
        "key_press" => {
            let key = action
                .key
                .as_ref()
                .ok_or_else(|| "key is required for key_press".to_string())?;
            if key.trim().is_empty() {
                return Err("key cannot be empty".to_string());
            }
        }
        other => {
            return Err(format!("unsupported input action: {other}"));
        }
    }

    Ok(())
}

pub(crate) fn push_ephemeral_frame(frames: &mut VecDeque<CaptureFrame>, frame: CaptureFrame) {
    frames.push_back(frame);
    while frames.len() > MAX_EPHEMERAL_FRAMES {
        let _ = frames.pop_front();
    }
}

pub(crate) fn push_ephemeral_vision_summary(
    summaries: &mut VecDeque<VisionSummary>,
    summary: VisionSummary,
) {
    summaries.push_back(summary);
    while summaries.len() > MAX_EPHEMERAL_VISION_SUMMARIES {
        let _ = summaries.pop_front();
    }
}

pub(crate) fn parse_vision_summary_output(frame: CaptureFrame, raw: &str) -> VisionSummary {
    let fallback = truncate_tail(raw.trim(), 512);
    let value = serde_json::from_str::<serde_json::Value>(raw).ok();
    let ui_state = value
        .as_ref()
        .and_then(|v| v.get("ui_state"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("UI state unavailable");
    let key_text = value
        .as_ref()
        .and_then(|v| v.get("key_text"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .unwrap_or("");
    let actionable_notes = value
        .as_ref()
        .and_then(|v| v.get("actionable_notes"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or(&fallback);
    let confidence = value
        .as_ref()
        .and_then(|v| v.get("confidence"))
        .and_then(|v| v.as_f64())
        .map(|v| v as f32)
        .unwrap_or(0.66)
        .clamp(0.0, 1.0);

    VisionSummary {
        id: format!("vision-{}-{}", frame.captured_at_ms, Uuid::new_v4()),
        captured_at_ms: frame.captured_at_ms,
        app_name: frame.app_name,
        window_title: frame.window_title,
        ui_state: truncate_tail(ui_state, 220),
        key_text: truncate_tail(key_text, 280),
        actionable_notes: truncate_tail(actionable_notes, 560),
        confidence,
    }
}

pub(crate) async fn persist_vision_summary(summary: VisionSummary) {
    let config = match Config::load_or_init().await {
        Ok(cfg) => cfg,
        Err(err) => {
            tracing::debug!("vision summary persistence skipped: config load failed: {err}");
            return;
        }
    };

    let embedder = Arc::from(memory::embeddings::create_embedding_provider(
        &config.memory.embedding_provider,
        config.api_key.as_deref(),
        &config.memory.embedding_model,
        config.memory.embedding_dimensions,
    ));
    let mem = match UnifiedMemory::new(
        &config.workspace_dir,
        embedder,
        config.memory.sqlite_open_timeout_secs,
    ) {
        Ok(mem) => mem,
        Err(err) => {
            tracing::debug!("vision summary persistence skipped: memory init failed: {err}");
            return;
        }
    };

    let content = match serde_json::to_string(&summary) {
        Ok(content) => content,
        Err(err) => {
            tracing::debug!("vision summary persistence skipped: serialization failed: {err}");
            return;
        }
    };

    let key = format!("screen_intelligence_{}", summary.id);
    let _ = mem
        .upsert_document(NamespaceDocumentInput {
            namespace: "background".to_string(),
            key: key.clone(),
            title: key,
            content,
            source_type: "screenshot".to_string(),
            priority: "medium".to_string(),
            tags: vec!["screen_intelligence".to_string()],
            metadata: serde_json::json!({}),
            category: "screen_intelligence".to_string(),
            session_id: None,
            document_id: None,
        })
        .await;
}

pub(crate) fn truncate_tail(text: &str, max_chars: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max_chars {
        return text.to_string();
    }
    chars[chars.len() - max_chars..].iter().collect()
}

pub(crate) fn generate_suggestions(
    context: &str,
    max_results: usize,
) -> Vec<AutocompleteSuggestion> {
    let trimmed = context.trim();
    let lower = trimmed.to_lowercase();

    let mut out = Vec::new();
    if lower.ends_with("thanks") || lower.ends_with("thank you") {
        out.push(AutocompleteSuggestion {
            value: " for your help!".to_string(),
            confidence: 0.89,
        });
    }
    if lower.contains("meeting") {
        out.push(AutocompleteSuggestion {
            value: " tomorrow at 10am works for me.".to_string(),
            confidence: 0.84,
        });
    }
    if lower.contains("ship") || lower.contains("release") {
        out.push(AutocompleteSuggestion {
            value: " after we pass QA and smoke tests.".to_string(),
            confidence: 0.81,
        });
    }

    if out.is_empty() {
        out.push(AutocompleteSuggestion {
            value: " Please share any constraints and I can refine this.".to_string(),
            confidence: 0.55,
        });
    }

    out.truncate(max_results);
    out
}
