use crate::openhuman::config::Config;
use crate::openhuman::memory::{self, NamespaceDocumentInput, UnifiedMemory};
use std::collections::VecDeque;
use std::sync::Arc;
use uuid::Uuid;

use super::limits::{MAX_CONTEXT_CHARS, MAX_EPHEMERAL_FRAMES, MAX_EPHEMERAL_VISION_SUMMARIES};
use super::types::{AutocompleteSuggestion, CaptureFrame, InputActionParams, VisionSummary};

/// Default confidence score used when the model does not provide one.
/// Applied consistently across both JSON and plain-text vision output branches.
const DEFAULT_VISION_CONFIDENCE: f32 = 0.8;

pub(crate) const VISION_MEMORY_NAMESPACE: &str = "background";
pub(crate) const VISION_MEMORY_SOURCE_TYPE: &str = "screenshot";
pub(crate) const VISION_MEMORY_CATEGORY: &str = "screen_intelligence";
pub(crate) const VISION_MEMORY_TAG: &str = "screen_intelligence";

#[derive(Debug, Clone)]
pub(crate) struct PersistVisionSummaryResult {
    pub namespace: String,
    pub key: String,
}

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
    // Deduplicate: skip if a summary with the same captured_at_ms already exists.
    // This prevents `vision_flush` from storing duplicates when called concurrently
    // with the processing worker channel path.
    if summaries
        .iter()
        .any(|s| s.captured_at_ms == summary.captured_at_ms)
    {
        tracing::debug!(
            "[screen_intelligence] skipping duplicate vision summary (captured_at_ms={})",
            summary.captured_at_ms
        );
        return;
    }
    summaries.push_back(summary);
    while summaries.len() > MAX_EPHEMERAL_VISION_SUMMARIES {
        let _ = summaries.pop_front();
    }
}

pub(crate) fn parse_vision_summary_output(frame: CaptureFrame, raw: &str) -> VisionSummary {
    let trimmed = raw.trim();

    // Try JSON first (backwards compat / mock testing).
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        let ui_state = value
            .get("ui_state")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        let key_text = value
            .get("key_text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        let actionable_notes = value
            .get("actionable_notes")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        let confidence = value
            .get("confidence")
            .and_then(|v| v.as_f64())
            .map(|v| v as f32)
            .unwrap_or(DEFAULT_VISION_CONFIDENCE)
            .clamp(0.0, 1.0);

        return VisionSummary {
            id: format!("vision-{}-{}", frame.captured_at_ms, Uuid::new_v4()),
            captured_at_ms: frame.captured_at_ms,
            app_name: frame.app_name,
            window_title: frame.window_title,
            ui_state: truncate_tail(ui_state, 500),
            key_text: truncate_tail(key_text, 2000),
            actionable_notes: truncate_tail(actionable_notes, 1000),
            confidence,
        };
    }

    // Plain text mode: first line = ui_state, second line = actionable_notes,
    // rest = key_text (the full content extraction).
    let mut lines = trimmed.lines();
    let ui_state = lines.next().unwrap_or("").trim().to_string();
    let actionable_notes = lines.next().unwrap_or("").trim().to_string();
    let key_text: String = lines.collect::<Vec<_>>().join("\n").trim().to_string();

    VisionSummary {
        id: format!("vision-{}-{}", frame.captured_at_ms, Uuid::new_v4()),
        captured_at_ms: frame.captured_at_ms,
        app_name: frame.app_name,
        window_title: frame.window_title,
        ui_state: truncate_tail(&ui_state, 500),
        key_text: truncate_tail(&key_text, 4000),
        actionable_notes: truncate_tail(&actionable_notes, 1000),
        confidence: DEFAULT_VISION_CONFIDENCE,
    }
}

pub(crate) async fn persist_vision_summary(
    summary: VisionSummary,
) -> Result<PersistVisionSummaryResult, String> {
    let config = match Config::load_or_init().await {
        Ok(cfg) => cfg,
        Err(err) => {
            let message = format!("config load failed: {err}");
            tracing::debug!(
                "[screen_intelligence] vision summary persistence skipped: {}",
                message
            );
            return Err(message);
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
            let message = format!("memory init failed: {err}");
            tracing::debug!(
                "[screen_intelligence] vision summary persistence skipped: {}",
                message
            );
            return Err(message);
        }
    };

    let ts = chrono::DateTime::from_timestamp_millis(summary.captured_at_ms)
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .unwrap_or_else(|| summary.captured_at_ms.to_string());
    let app = summary.app_name.as_deref().unwrap_or("Unknown");
    let window = summary.window_title.as_deref().unwrap_or("");

    let title = format!("Screen capture — {} — {}", app, ts);

    // YAML frontmatter for metadata, body is clean markdown content.
    // Limitation: escaping is best-effort — only double-quotes and newlines are
    // escaped. Values containing YAML-special characters like `:`, `{`, `}`, `[`,
    // `]`, `#`, `|`, `>`, `&`, `*` may still produce invalid YAML in edge cases.
    let yaml_escape = |s: &str| -> String {
        s.replace('"', "\\\"").replace('\n', "\\n").replace('\r', "")
    };
    let mut content = String::from("---\n");
    content.push_str(&format!("app: \"{}\"\n", yaml_escape(app)));
    if !window.is_empty() {
        content.push_str(&format!("window: \"{}\"\n", yaml_escape(window)));
    }
    content.push_str(&format!("captured: \"{}\"\n", ts));
    content.push_str(&format!("captured_ms: {}\n", summary.captured_at_ms));
    content.push_str(&format!("confidence: {:.2}\n", summary.confidence));
    content.push_str(&format!("id: \"{}\"\n", summary.id));
    content.push_str("---\n\n");

    // key_text = synthesized summary (the main document body)
    if !summary.key_text.is_empty() {
        content.push_str(&format!("{}\n", summary.key_text));
    }

    let key = format!("screen_intelligence_{}", summary.id);
    mem.upsert_document(NamespaceDocumentInput {
        namespace: VISION_MEMORY_NAMESPACE.to_string(),
        key: key.clone(),
        title,
        content,
        source_type: VISION_MEMORY_SOURCE_TYPE.to_string(),
        priority: "medium".to_string(),
        tags: vec![VISION_MEMORY_TAG.to_string()],
        metadata: serde_json::json!({}),
        category: VISION_MEMORY_CATEGORY.to_string(),
        session_id: None,
        document_id: None,
    })
    .await
    .map_err(|err| format!("memory upsert failed: {err}"))?;

    tracing::debug!(
        "[screen_intelligence] persisted vision summary into unified memory (namespace={} key={} app={:?} captured_at_ms={})",
        VISION_MEMORY_NAMESPACE,
        key,
        summary.app_name,
        summary.captured_at_ms
    );

    Ok(PersistVisionSummaryResult {
        namespace: VISION_MEMORY_NAMESPACE.to_string(),
        key,
    })
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
