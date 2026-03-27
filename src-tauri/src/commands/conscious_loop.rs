//! Conscious Loop — periodic background process that digests all skill memory
//! into structured actionable items.
//!
//! # Flow
//!
//! Every 5 minutes (and on manual trigger):
//! 1. Get active skill IDs from the runtime engine via `engine.all_tools()`
//! 2. Recall memory for each skill via `recall_skill_context`
//! 3. Send assembled contexts to the LLM with the CONSCIOUS_LOOP.md prompt
//! 4. Log the full LLM response
//! 5. Parse JSON array of `ExtractedActionable` items
//! 6. Store each item in the `conscious` memory namespace via `store_skill_sync`
//! 7. Emit completion events to the frontend

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tauri::{Emitter, Manager};

use crate::memory::MemoryState;

// ─── Event types (Rust → frontend) ──────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct ConsciousLoopStartedEvent {
    pub run_id: String,
    pub timestamp: u64,
    pub skill_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConsciousLoopCompletedEvent {
    pub run_id: String,
    pub actionable_count: usize,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConsciousLoopErrorEvent {
    pub run_id: String,
    pub message: String,
    pub error_type: String,
}

// ─── LLM response types ───────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<ChatCompletionChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionChoice {
    message: ChatCompletionMessage,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionMessage {
    content: Option<String>,
}

// ─── Actionable item ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Serialize)]
pub struct ExtractedActionable {
    pub title: String,
    pub description: String,
    pub source: String,
    pub priority: String,
    pub actionable: bool,
    pub requires_confirmation: bool,
    pub has_complex_action: bool,
    pub source_label: String,
}

// ─── Constants ────────────────────────────────────────────────────────────────

const CONSCIOUS_LOOP_INTERVAL_SECS: u64 = 300; // 5 minutes
const CONSCIOUS_LOOP_INITIAL_DELAY_SECS: u64 = 60; // allow skills to boot
const INFERENCE_TIMEOUT_SECS: u64 = 120;
const DEFAULT_MODEL: &str = "neocortex-mk1";
const CONSCIOUS_SKILL_ID: &str = "conscious";
const CONSCIOUS_INTEGRATION_ID: &str = "actionables";
const FALLBACK_PROMPT: &str = r#"You are the conscious awareness layer of OpenHuman. Analyze the recalled memory contexts and extract actionable items. Return a JSON array where each item has: title, description, source (email|calendar|telegram|ai_insight|system|trading|security), priority (critical|important|normal), actionable (bool), requires_confirmation (bool), has_complex_action (bool), source_label. Return ONLY the JSON array, no markdown fences. If nothing is found return []."#;

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// Deterministic document ID from title + source to enable deduplication.
fn document_id_for(title: &str, source: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(title.as_bytes());
    hasher.update(b":");
    hasher.update(source.as_bytes());
    format!("conscious-{:x}", hasher.finalize())
}

/// Find the `ai/` directory for loading prompt files (same logic as `chat.rs`).
fn find_ai_directory(app: &tauri::AppHandle) -> Option<std::path::PathBuf> {
    if let Ok(resource_dir) = app.path().resource_dir() {
        let ai_dir = resource_dir.join("ai");
        if ai_dir.is_dir() {
            return Some(ai_dir);
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        let root_dev = cwd.join("rust-core").join("ai");
        if root_dev.is_dir() {
            return Some(root_dev);
        }
        if let Some(src_tauri_dev) = cwd.parent().map(|p| p.join("rust-core").join("ai")) {
            if src_tauri_dev.is_dir() {
                return Some(src_tauri_dev);
            }
        }
        let fallback = cwd.join("ai");
        if fallback.is_dir() {
            return Some(fallback);
        }
        let src_tauri_legacy = cwd.join("src-tauri").join("ai");
        if src_tauri_legacy.is_dir() {
            return Some(src_tauri_legacy);
        }
        if let Some(legacy) = cwd.parent().map(|p| p.join("ai")) {
            if legacy.is_dir() {
                return Some(legacy);
            }
        }
    }
    None
}

/// Load CONSCIOUS_LOOP.md from the ai/ directory, falling back to the hardcoded
/// prompt if the file is missing.
fn load_conscious_prompt(app: &tauri::AppHandle) -> String {
    if let Some(dir) = find_ai_directory(app) {
        let path = dir.join("CONSCIOUS_LOOP.md");
        if let Ok(content) = std::fs::read_to_string(&path) {
            log::info!("[conscious_loop] Loaded prompt from {}", path.display());
            return content;
        }
        log::warn!(
            "[conscious_loop] CONSCIOUS_LOOP.md not found at {} — using fallback prompt",
            path.display()
        );
    } else {
        log::warn!("[conscious_loop] ai/ directory not found — using fallback prompt");
    }
    FALLBACK_PROMPT.to_string()
}

// ─── Core logic ───────────────────────────────────────────────────────────────

/// Inner implementation — runs one conscious loop pass.
pub async fn conscious_loop_run_inner(
    app: tauri::AppHandle,
    auth_token: String,
    backend_url: String,
    model: String,
    memory_client: Arc<crate::memory::MemoryClient>,
) {
    let run_id = uuid::Uuid::new_v4().to_string();
    let started_at = std::time::Instant::now();
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // ── Step 1: Get active skill IDs from runtime engine ─────────────────
    let engine_state = app.try_state::<Arc<crate::runtime::qjs_engine::RuntimeEngine>>();
    let skill_ids: Vec<String> = if let Some(engine) = engine_state {
        let unique: std::collections::HashSet<String> = engine
            .all_tools()
            .into_iter()
            .map(|(skill_id, _)| skill_id)
            .collect();
        let mut ids: Vec<String> = unique.into_iter().collect();
        ids.sort();
        ids
    } else {
        log::warn!("[conscious_loop] run_id={run_id} — RuntimeEngine not available, skipping");
        return;
    };

    log::info!(
        "[conscious_loop] run_id={run_id} starting — skill_ids={:?}",
        skill_ids
    );

    let _ = app.emit(
        "conscious_loop:started",
        ConsciousLoopStartedEvent {
            run_id: run_id.clone(),
            timestamp,
            skill_ids: skill_ids.clone(),
        },
    );

    if skill_ids.is_empty() {
        log::info!(
            "[conscious_loop] run_id={run_id} — no active skills, emitting completed with 0 items"
        );
        let _ = app.emit(
            "conscious_loop:completed",
            ConsciousLoopCompletedEvent {
                run_id,
                actionable_count: 0,
                duration_ms: started_at.elapsed().as_millis() as u64,
            },
        );
        return;
    }

    // ── Step 2: Recall memory for each skill ─────────────────────────────
    let mut recalled_contexts: Vec<(String, String)> = Vec::new();
    for skill_id in &skill_ids {
        match memory_client
            .recall_skill_context(skill_id, skill_id, 10)
            .await
        {
            Ok(Some(ctx)) => {
                let ctx_str = if let Some(s) = ctx.as_str() {
                    s.to_string()
                } else {
                    ctx.to_string()
                };
                if !ctx_str.is_empty() {
                    log::info!(
                        "[conscious_loop] run_id={run_id} skill={skill_id} recalled {} chars",
                        ctx_str.len()
                    );
                    recalled_contexts.push((skill_id.to_string(), ctx_str));
                } else {
                    log::info!("[conscious_loop] run_id={run_id} skill={skill_id} — empty recall");
                }
            }
            Ok(None) => {
                log::info!("[conscious_loop] run_id={run_id} skill={skill_id} — no memory context");
            }
            Err(e) => {
                log::warn!(
                    "[conscious_loop] run_id={run_id} skill={skill_id} recall failed: {e} — skipping"
                );
            }
        }
    }

    if recalled_contexts.is_empty() {
        log::info!("[conscious_loop] run_id={run_id} — all skill recalls empty, emitting completed with 0 items");
        let _ = app.emit(
            "conscious_loop:completed",
            ConsciousLoopCompletedEvent {
                run_id,
                actionable_count: 0,
                duration_ms: started_at.elapsed().as_millis() as u64,
            },
        );
        return;
    }

    // ── Step 3: Load prompt ───────────────────────────────────────────────
    let prompt = load_conscious_prompt(&app);

    // ── Step 4: Build user message from recalled contexts ─────────────────
    let assembled = recalled_contexts
        .iter()
        .map(|(skill_id, ctx)| {
            format!(
                "[{}_CONTEXT]\n{}\n[/{}_CONTEXT]",
                skill_id.to_uppercase(),
                ctx,
                skill_id.to_uppercase()
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    let request_body = serde_json::json!({
        "model": model,
        "messages": [
            {"role": "system", "content": prompt},
            {"role": "user", "content": assembled},
        ],
    });

    log::info!(
        "[conscious_loop] run_id={run_id} sending inference request — model={model}, contexts={}",
        recalled_contexts.len()
    );
    log::info!(
        "[conscious_loop] Request body: {}",
        serde_json::to_string_pretty(&request_body).unwrap_or_default()
    );

    // ── Step 5: Call inference ────────────────────────────────────────────
    let client = reqwest::Client::new();
    let url = format!("{backend_url}/openai/v1/chat/completions");

    let inference_result = tokio::time::timeout(
        std::time::Duration::from_secs(INFERENCE_TIMEOUT_SECS),
        client
            .post(&url)
            .header("Authorization", format!("Bearer {auth_token}"))
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send(),
    )
    .await;

    let response_text = match inference_result {
        Ok(Ok(resp)) if resp.status().is_success() => match resp.text().await {
            Ok(text) => text,
            Err(e) => {
                let msg = format!("Failed to read inference response body: {e}");
                log::warn!("[conscious_loop] run_id={run_id} {msg}");
                let _ = app.emit(
                    "conscious_loop:error",
                    ConsciousLoopErrorEvent {
                        run_id,
                        message: msg,
                        error_type: "inference_read_error".to_string(),
                    },
                );
                return;
            }
        },
        Ok(Ok(resp)) => {
            let status = resp.status();
            let msg = format!("Inference API returned HTTP {status}");
            log::warn!("[conscious_loop] run_id={run_id} {msg}");
            let _ = app.emit(
                "conscious_loop:error",
                ConsciousLoopErrorEvent {
                    run_id,
                    message: msg,
                    error_type: "inference_http_error".to_string(),
                },
            );
            return;
        }
        Ok(Err(e)) => {
            let msg = format!("Inference request failed: {e}");
            log::warn!("[conscious_loop] run_id={run_id} {msg}");
            let _ = app.emit(
                "conscious_loop:error",
                ConsciousLoopErrorEvent {
                    run_id,
                    message: msg,
                    error_type: "inference_error".to_string(),
                },
            );
            return;
        }
        Err(_) => {
            let msg = format!("Inference request timed out after {INFERENCE_TIMEOUT_SECS}s");
            log::warn!("[conscious_loop] run_id={run_id} {msg}");
            let _ = app.emit(
                "conscious_loop:error",
                ConsciousLoopErrorEvent {
                    run_id,
                    message: msg,
                    error_type: "inference_timeout".to_string(),
                },
            );
            return;
        }
    };

    // ── Step 6: Extract content from chat completion wrapper ──────────────
    let content = match serde_json::from_str::<ChatCompletionResponse>(&response_text) {
        Ok(completion) => completion
            .choices
            .into_iter()
            .next()
            .and_then(|c| c.message.content)
            .unwrap_or_default(),
        Err(e) => {
            log::warn!(
                "[conscious_loop] run_id={run_id} failed to parse completion wrapper: {e}\nRaw: {response_text}"
            );
            // Try treating the whole response as the content directly
            response_text.clone()
        }
    };

    // ── Step 7: Log full LLM response ────────────────────────────────────
    log::info!("[conscious_loop] run_id={run_id} LLM response: {content}");

    // ── Step 8: Parse JSON array of actionables ───────────────────────────
    let actionables: Vec<ExtractedActionable> = match serde_json::from_str(&content) {
        Ok(items) => items,
        Err(e) => {
            let msg = format!("Failed to parse actionables JSON: {e}");
            log::warn!("[conscious_loop] run_id={run_id} {msg}\nRaw content: {content}");
            let _ = app.emit(
                "conscious_loop:error",
                ConsciousLoopErrorEvent {
                    run_id,
                    message: msg,
                    error_type: "json_parse_error".to_string(),
                },
            );
            return;
        }
    };

    log::info!(
        "[conscious_loop] run_id={run_id} extracted {} actionable item(s)",
        actionables.len()
    );

    // ── Step 9: Insert each actionable into memory ────────────────────────
    let mut insert_count = 0usize;
    for item in &actionables {
        let doc_id = document_id_for(&item.title, &item.source);
        let content_json = match serde_json::to_string_pretty(item) {
            Ok(s) => s,
            Err(e) => {
                log::warn!(
                    "[conscious_loop] run_id={run_id} failed to serialize item '{}': {e}",
                    item.title
                );
                continue;
            }
        };

        match memory_client
            .store_skill_sync(
                CONSCIOUS_SKILL_ID,
                CONSCIOUS_INTEGRATION_ID,
                &item.title,
                &content_json,
                None,
                None,
                None,
                None,
                None,
                Some(doc_id.clone()),
            )
            .await
        {
            Ok(()) => {
                log::info!(
                    "[conscious_loop] run_id={run_id} inserted doc_id={doc_id} title={:?}",
                    item.title
                );
                insert_count += 1;
            }
            Err(e) => {
                log::warn!(
                    "[conscious_loop] run_id={run_id} failed to insert doc_id={doc_id}: {e} — continuing"
                );
            }
        }
    }

    // ── Step 10: Emit completed ───────────────────────────────────────────
    let duration_ms = started_at.elapsed().as_millis() as u64;
    log::info!(
        "[conscious_loop] run_id={run_id} completed — inserted={insert_count}/{} duration={duration_ms}ms",
        actionables.len()
    );
    let _ = app.emit(
        "conscious_loop:completed",
        ConsciousLoopCompletedEvent {
            run_id,
            actionable_count: insert_count,
            duration_ms,
        },
    );
}

// ─── Tauri command ────────────────────────────────────────────────────────────

/// Manually trigger a conscious loop run from the frontend.
#[tauri::command]
pub async fn conscious_loop_run(
    app: tauri::AppHandle,
    auth_token: String,
    backend_url: String,
    model: Option<String>,
    memory_state: tauri::State<'_, MemoryState>,
) -> Result<(), String> {
    let memory_client = {
        match memory_state.0.lock() {
            Ok(guard) => guard.clone(),
            Err(e) => {
                return Err(format!("Failed to lock memory state: {e}"));
            }
        }
    };
    let memory_client = memory_client.ok_or_else(|| {
        "Memory client not initialized — call init_memory_client first".to_string()
    })?;

    let model = model.unwrap_or_else(|| {
        std::env::var("OPENHUMAN_CONSCIOUS_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string())
    });

    tauri::async_runtime::spawn(async move {
        conscious_loop_run_inner(app, auth_token, backend_url, model, memory_client).await;
    });

    Ok(())
}

// ─── Periodic timer ───────────────────────────────────────────────────────────

/// Periodic timer spawned from `lib.rs` setup. Runs every 5 minutes.
/// Does nothing if the memory client is not yet initialized or no auth token is present.
pub async fn conscious_loop_timer(app: tauri::AppHandle) {
    log::info!(
        "[conscious_loop] Timer starting — initial delay {}s",
        CONSCIOUS_LOOP_INITIAL_DELAY_SECS
    );
    tokio::time::sleep(std::time::Duration::from_secs(
        CONSCIOUS_LOOP_INITIAL_DELAY_SECS,
    ))
    .await;

    let mut interval =
        tokio::time::interval(std::time::Duration::from_secs(CONSCIOUS_LOOP_INTERVAL_SECS));

    loop {
        interval.tick().await;

        // Check memory client
        let memory_client: Option<crate::memory::MemoryClientRef> = {
            let memory_state = app.try_state::<MemoryState>();
            match memory_state {
                Some(state) => {
                    match state.0.lock() {
                        Ok(guard) => guard.clone(),
                        Err(_) => {
                            log::warn!("[conscious_loop] Timer: failed to lock memory state — skipping tick");
                            continue;
                        }
                    }
                }
                None => {
                    log::warn!(
                        "[conscious_loop] Timer: MemoryState not registered — skipping tick"
                    );
                    continue;
                }
            }
        };

        let Some(memory_client) = memory_client else {
            log::info!("[conscious_loop] Timer: memory client not initialized — skipping tick");
            continue;
        };

        // Get auth token from session service
        let auth_token = match crate::commands::auth::SESSION_SERVICE.get_token() {
            Some(token) => token,
            None => {
                log::info!("[conscious_loop] Timer: no auth token — skipping tick");
                continue;
            }
        };

        let backend_url = crate::utils::config::get_backend_url();
        let model = std::env::var("OPENHUMAN_CONSCIOUS_MODEL")
            .unwrap_or_else(|_| DEFAULT_MODEL.to_string());

        log::info!("[conscious_loop] Timer: firing periodic run");
        conscious_loop_run_inner(app.clone(), auth_token, backend_url, model, memory_client).await;
    }
}
