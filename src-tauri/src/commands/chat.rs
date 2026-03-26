//! Tauri commands for the Rust-side conversation orchestration.
//!
//! Moves the agentic loop (context injection, inference API calls, tool execution)
//! from `Conversations.tsx` into Rust, so the frontend becomes a thin renderer.
//!
//! # Command overview
//!
//! - `chat_send`   — spawn the agentic loop in a background task; returns immediately.
//! - `chat_cancel` — cancel an in-flight `chat_send` by thread ID.
//!
//! # Event protocol (Rust → frontend)
//!
//! | Event name        | Payload type          | When emitted                    |
//! |-------------------|-----------------------|---------------------------------|
//! | `chat:tool_call`  | `ChatToolCallEvent`   | Before a tool is executed       |
//! | `chat:tool_result`| `ChatToolResultEvent` | After a tool completes          |
//! | `chat:done`       | `ChatDoneEvent`       | Agent loop finishes (success)   |
//! | `chat:error`      | `ChatErrorEvent`      | Any error during the loop       |

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tauri::{Emitter, Manager};
use tokio_util::sync::CancellationToken;

use crate::commands::memory::MemoryState;

// ─── Constants ───────────────────────────────────────────────────────────────

const MAX_TOOL_ROUNDS: u32 = 5;
const INFERENCE_TIMEOUT_SECS: u64 = 120;
const TOOL_TIMEOUT_SECS: u64 = 60;
const MAX_CONTEXT_CHARS: usize = 20_000;

/// Names and order of the OpenClaw workspace files.
const OPENCLAW_FILES: &[&str] = &[
    "SOUL.md",
    "IDENTITY.md",
    "AGENTS.md",
    "USER.md",
    "BOOTSTRAP.md",
    "MEMORY.md",
    "TOOLS.md",
];

// ─── Input types (frontend → Rust) ───────────────────────────────────────────

/// A single message in the conversation history, sent from the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessagePayload {
    pub role: String, // "user" | "assistant" | "system" | "tool"
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCallPayload>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallPayload {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: String, // always "function"
    pub function: ToolCallFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallFunction {
    pub name: String,
    pub arguments: String, // JSON string
}

/// Parameters for the `chat_send` Tauri command.
#[derive(Debug, Clone, Deserialize)]
pub struct ChatSendParams {
    pub thread_id: String,
    pub message: String,
    pub model: String,
    pub auth_token: String,
    pub backend_url: String,
    pub messages: Vec<ChatMessagePayload>,
    #[serde(default)]
    pub notion_context: Option<String>,
}

// ─── Event payload types (Rust → frontend) ───────────────────────────────────

/// Emitted when the agent invokes a tool.
#[derive(Debug, Clone, Serialize)]
pub struct ChatToolCallEvent {
    pub thread_id: String,
    pub tool_name: String,
    pub skill_id: String,
    pub args: serde_json::Value,
    pub round: u32,
}

/// Emitted when a tool completes execution.
#[derive(Debug, Clone, Serialize)]
pub struct ChatToolResultEvent {
    pub thread_id: String,
    pub tool_name: String,
    pub skill_id: String,
    pub output: String,
    pub success: bool,
    pub round: u32,
}

/// Emitted when the agent loop completes successfully.
#[derive(Debug, Clone, Serialize)]
pub struct ChatDoneEvent {
    pub thread_id: String,
    pub full_response: String,
    pub rounds_used: u32,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
}

/// Emitted when an error occurs during the agent loop.
#[derive(Debug, Clone, Serialize)]
pub struct ChatErrorEvent {
    pub thread_id: String,
    pub message: String,
    /// "network" | "timeout" | "tool_error" | "inference" | "cancelled"
    pub error_type: String,
    pub round: Option<u32>,
}

// ─── Backend API response types ───────────────────────────────────────────────

/// OpenAI-compatible chat completion response.
#[derive(Debug, Clone, Deserialize)]
pub struct ChatCompletionResponse {
    #[allow(dead_code)]
    pub id: String,
    #[allow(dead_code)]
    pub model: String,
    pub choices: Vec<ChatCompletionChoice>,
    #[serde(default)]
    pub usage: Option<CompletionUsage>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChatCompletionChoice {
    #[allow(dead_code)]
    pub index: u32,
    pub message: ChatCompletionMessage,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChatCompletionMessage {
    #[allow(dead_code)]
    pub role: String,
    pub content: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<ToolCallPayload>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CompletionUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    #[allow(dead_code)]
    pub total_tokens: u64,
}

// ─── Internal state ───────────────────────────────────────────────────────────

/// Tracks in-flight chat requests for cancellation support.
pub struct ChatState {
    active_requests: RwLock<HashMap<String, CancellationToken>>,
}

impl ChatState {
    pub fn new() -> Self {
        Self {
            active_requests: RwLock::new(HashMap::new()),
        }
    }

    pub fn register(&self, thread_id: &str) -> CancellationToken {
        let token = CancellationToken::new();
        self.active_requests
            .write()
            .insert(thread_id.to_string(), token.clone());
        token
    }

    pub fn cancel(&self, thread_id: &str) -> bool {
        if let Some(token) = self.active_requests.write().remove(thread_id) {
            token.cancel();
            true
        } else {
            false
        }
    }

    pub fn remove(&self, thread_id: &str) {
        self.active_requests.write().remove(thread_id);
    }
}

// ─── AI config loader ─────────────────────────────────────────────────────────

/// In-memory cache for AI config content.
/// Populated on first call; cleared only on app restart.
static AI_CONFIG_CACHE: once_cell::sync::Lazy<parking_lot::RwLock<Option<String>>> =
    once_cell::sync::Lazy::new(|| parking_lot::RwLock::new(None));

/// Load all AI config files and build the OpenClaw context string.
///
/// Tries these locations in order:
/// 1. Tauri resource directory (production builds — files bundled via `tauri.conf.json` resources)
/// 2. `{cwd}/../ai/` (dev mode — project root relative to `src-tauri/`)
/// 3. `{cwd}/ai/` (fallback)
///
/// Returns an empty string if no files are found (non-fatal).
fn load_openclaw_context(app: &tauri::AppHandle) -> String {
    // Check cache first
    if let Some(cached) = AI_CONFIG_CACHE.read().as_ref() {
        return cached.clone();
    }

    let mut sections: Vec<String> = Vec::new();

    if let Some(dir) = find_ai_directory(app) {
        for filename in OPENCLAW_FILES {
            let path = dir.join(filename);
            if let Ok(content) = std::fs::read_to_string(&path) {
                let trimmed = content.trim().to_string();
                if has_meaningful_content(&trimmed) {
                    sections.push(format!("### {}\n\n{}", filename, trimmed));
                }
            }
        }
    }

    if sections.is_empty() {
        log::warn!("[chat] No AI config files found — proceeding without context");
        let empty = String::new();
        *AI_CONFIG_CACHE.write() = Some(empty.clone());
        return empty;
    }

    let mut context = format!("## Project Context\n\n{}", sections.join("\n\n---\n\n"));
    if context.len() > MAX_CONTEXT_CHARS {
        context.truncate(MAX_CONTEXT_CHARS);
        context.push_str("\n\n[...truncated]");
    }

    *AI_CONFIG_CACHE.write() = Some(context.clone());
    context
}

/// Find the `ai/` directory. Returns `None` if not found.
fn find_ai_directory(app: &tauri::AppHandle) -> Option<std::path::PathBuf> {
    // 1. Try resource dir first (production builds)
    if let Ok(resource_dir) = app.path().resource_dir() {
        let ai_dir: std::path::PathBuf = resource_dir.join("ai");
        if ai_dir.is_dir() {
            log::info!(
                "[chat] Using AI config from resource dir: {}",
                ai_dir.display()
            );
            return Some(ai_dir);
        }
    }

    // 2. Try cwd/../ai/ (dev mode; cwd is src-tauri/)
    if let Ok(cwd) = std::env::current_dir() {
        let dev_dir = cwd.parent().map(|p| p.join("ai"));
        if let Some(ref dir) = dev_dir {
            if dir.is_dir() {
                log::info!(
                    "[chat] Using AI config from dev dir: {}",
                    dir.display()
                );
                return dev_dir;
            }
        }
        // 3. Try cwd/ai/ (fallback)
        let fallback = cwd.join("ai");
        if fallback.is_dir() {
            log::info!(
                "[chat] Using AI config from fallback dir: {}",
                fallback.display()
            );
            return Some(fallback);
        }
    }

    log::warn!("[chat] No AI config directory found");
    None
}

/// Check if file content has meaningful data (not just a TODO template).
fn has_meaningful_content(content: &str) -> bool {
    let lines: Vec<&str> = content.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.len() <= 3 {
        return false;
    }
    let first_content = lines.iter().find(|l| !l.starts_with('#'));
    if let Some(line) = first_content {
        if line.trim().starts_with("TODO:") {
            return false;
        }
    }
    true
}

// ─── Tool discovery (desktop only) ───────────────────────────────────────────

/// Build OpenAI-format tool definitions from the Rust skill registry.
/// Tool names are namespaced as `{skill_id}__{tool_name}`.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn discover_tools(
    engine: &crate::runtime::qjs_engine::RuntimeEngine,
) -> Vec<serde_json::Value> {
    let raw_tools = engine.all_tools();
    raw_tools
        .into_iter()
        .map(|(skill_id, tool)| {
            serde_json::json!({
                "type": "function",
                "function": {
                    "name": format!("{}__{}", skill_id, tool.name),
                    "description": tool.description,
                    "parameters": tool.input_schema,
                }
            })
        })
        .collect()
}

// ─── Helper ───────────────────────────────────────────────────────────────────

/// Parse a namespaced tool name `"skillId__toolName"` into `(skill_id, tool_name)`.
fn parse_tool_name(full_name: &str) -> (String, String) {
    if let Some(idx) = full_name.find("__") {
        (
            full_name[..idx].to_string(),
            full_name[idx + 2..].to_string(),
        )
    } else {
        (String::new(), full_name.to_string())
    }
}

// ─── Commands ────────────────────────────────────────────────────────────────

/// Start an agentic conversation loop in a background task.
///
/// Returns `Ok(())` immediately after spawning; the result is delivered via
/// `chat:done` or `chat:error` Tauri events.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
#[tauri::command]
pub async fn chat_send(
    app: tauri::AppHandle,
    thread_id: String,
    message: String,
    model: String,
    auth_token: String,
    backend_url: String,
    messages: Vec<ChatMessagePayload>,
    notion_context: Option<String>,
    engine: tauri::State<'_, Arc<crate::runtime::qjs_engine::RuntimeEngine>>,
    memory_state: tauri::State<'_, MemoryState>,
    chat_state: tauri::State<'_, Arc<ChatState>>,
) -> Result<(), String> {
    // Register cancellation token for this thread
    let cancel = chat_state.register(&thread_id);

    // Clone values that need to cross the spawn boundary
    let app_clone = app.clone();
    let thread_id_clone = thread_id.clone();
    let chat_state_arc = chat_state.inner().clone();
    let engine_arc = engine.inner().clone();

    // Clone the MemoryClientRef (Option<Arc<MemoryClient>>) out of the Mutex
    let memory_client: Option<crate::memory::MemoryClientRef> = {
        match memory_state.0.lock() {
            Ok(guard) => guard.clone(),
            Err(e) => {
                log::warn!("[chat] Failed to lock memory state: {e}");
                None
            }
        }
    };

    tauri::async_runtime::spawn(async move {
        let result = chat_send_inner(
            &app_clone,
            &thread_id_clone,
            &message,
            &model,
            &auth_token,
            &backend_url,
            messages,
            notion_context,
            &engine_arc,
            memory_client,
            &cancel,
        )
        .await;

        // Clean up the cancellation token
        chat_state_arc.remove(&thread_id_clone);

        if let Err(e) = result {
            let _ = app_clone.emit(
                "chat:error",
                ChatErrorEvent {
                    thread_id: thread_id_clone,
                    message: e,
                    error_type: "inference".to_string(),
                    round: None,
                },
            );
        }
    });

    Ok(())
}

/// Mobile stub — tool execution is not supported on Android/iOS.
#[cfg(any(target_os = "android", target_os = "ios"))]
#[tauri::command]
pub async fn chat_send(
    app: tauri::AppHandle,
    thread_id: String,
    message: String,
    model: String,
    auth_token: String,
    backend_url: String,
    messages: Vec<ChatMessagePayload>,
    notion_context: Option<String>,
    memory_state: tauri::State<'_, MemoryState>,
    chat_state: tauri::State<'_, Arc<ChatState>>,
) -> Result<(), String> {
    // Register cancellation token for this thread
    let cancel = chat_state.register(&thread_id);

    let app_clone = app.clone();
    let thread_id_clone = thread_id.clone();
    let chat_state_arc = chat_state.inner().clone();

    let memory_client: Option<crate::memory::MemoryClientRef> = {
        match memory_state.0.lock() {
            Ok(guard) => guard.clone(),
            Err(e) => {
                log::warn!("[chat] Failed to lock memory state: {e}");
                None
            }
        }
    };

    tauri::async_runtime::spawn(async move {
        let result = chat_send_mobile(
            &app_clone,
            &thread_id_clone,
            &message,
            &model,
            &auth_token,
            &backend_url,
            messages,
            notion_context,
            memory_client,
            &cancel,
        )
        .await;

        chat_state_arc.remove(&thread_id_clone);

        if let Err(e) = result {
            let _ = app_clone.emit(
                "chat:error",
                ChatErrorEvent {
                    thread_id: thread_id_clone,
                    message: e,
                    error_type: "inference".to_string(),
                    round: None,
                },
            );
        }
    });

    Ok(())
}

/// Cancel an in-flight `chat_send` request by thread ID.
/// Returns `true` if a request was found and cancelled, `false` otherwise.
#[tauri::command]
pub fn chat_cancel(thread_id: String, chat_state: tauri::State<'_, Arc<ChatState>>) -> bool {
    log::info!("[chat] cancel requested for thread={}", thread_id);
    chat_state.cancel(&thread_id)
}

// ─── Inner implementation (desktop) ──────────────────────────────────────────

/// Agentic loop — runs in a background task on desktop.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
async fn chat_send_inner(
    app: &tauri::AppHandle,
    thread_id: &str,
    user_message: &str,
    model: &str,
    auth_token: &str,
    backend_url: &str,
    history: Vec<ChatMessagePayload>,
    notion_context: Option<String>,
    engine: &crate::runtime::qjs_engine::RuntimeEngine,
    memory_client: Option<crate::memory::MemoryClientRef>,
    cancel: &CancellationToken,
) -> Result<(), String> {
    log::info!("[chat] Backend URL: {}", backend_url);

    let client = reqwest::Client::builder()
        .use_rustls_tls()
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))?;

    // ── Step 1: Load AI context ─────────────────────────────────────────
    let openclaw_context = load_openclaw_context(app);

    // ── Step 2: Recall memory context ───────────────────────────────────
    log::info!("[chat] Recalling conversation memory (thread_id={thread_id})");
    let memory_context: Option<String> = if let Some(ref mem) = memory_client {
        match mem
            .recall_skill_context("conversations", thread_id, 10)
            .await
        {
            Ok(ctx) => {
                log::info!(
                    "[chat] Conversation memory recall: has_data={}, len={}",
                    ctx.is_some(),
                    ctx.as_ref().map(|ctx| ctx.to_string().len()).unwrap_or(0)
                );
                ctx.map(|ctx| ctx.to_string())
            }
            Err(e) => {
                log::warn!("[chat] Conversation memory recall failed: {}", e);
                None
            }
        }
    } else {
        log::info!("[chat] No memory client — skipping conversation memory recall");
        None
    };

    // ── Step 2b: Recall skill contexts ──────────────────────────────────
    let skill_ids: std::collections::HashSet<String> = engine
        .all_tools()
        .into_iter()
        .map(|(skill_id, _)| skill_id)
        .collect();

    log::info!("[chat] Recalling skill contexts for {} skill(s): {:?}", skill_ids.len(), skill_ids);

    let mut skill_contexts: Vec<String> = Vec::new();
    for sid in &skill_ids {
        if let Some(ref mem) = memory_client {
            log::info!("[chat] Recalling memory for skill={sid}");
            match mem.recall_skill_context(sid, sid, 10).await {
                Ok(Some(ctx)) => {
                    log::debug!("[chat] Skill memory content (skill={sid}):\n{}", ctx);
                    skill_contexts.push(format!(
                        "[{}_CONTEXT]\n{}\n[/{}_CONTEXT]",
                        sid.to_uppercase(),
                        ctx,
                        sid.to_uppercase()
                    ));
                }
                Ok(None) => {
                    log::info!("[chat] Skill memory recall: no data for skill={sid}");
                }
                Err(e) => {
                    log::warn!("[chat] Skill memory recall failed for skill={sid}: {}", e);
                }
            }
        }
    }

    log::info!(
        "[chat] Context assembly: conversation_memory={}, skill_contexts={}",
        memory_context.is_some(),
        skill_contexts.len()
    );

    // ── Step 3: Build processed user message ────────────────────────────
    let mut processed = user_message.to_string();

    if !openclaw_context.is_empty() {
        processed = format!("{}\n\nUser message: {}", openclaw_context, processed);
    }

    if let Some(ref mem) = memory_context {
        processed = format!(
            "[MEMORY_CONTEXT]\n{}\n[/MEMORY_CONTEXT]\n\n{}",
            mem, processed
        );
    }

    if !skill_contexts.is_empty() {
        processed = format!("{}\n\n{}", skill_contexts.join("\n\n"), processed);
    }

    if let Some(ref notion) = notion_context {
        processed = format!("{}\n\n{}", notion, processed);
    }

    // ── Step 4: Build chat messages array ────────────────────────────────
    let mut loop_messages: Vec<serde_json::Value> = history
        .iter()
        .map(|m| {
            let mut obj = serde_json::json!({
                "role": m.role,
                "content": m.content,
            });
            if let Some(ref tc) = m.tool_calls {
                obj["tool_calls"] = serde_json::to_value(tc).unwrap_or_default();
            }
            if let Some(ref id) = m.tool_call_id {
                obj["tool_call_id"] = serde_json::Value::String(id.clone());
            }
            obj
        })
        .collect();

    // Append the current user message (with injected context)
    loop_messages.push(serde_json::json!({
        "role": "user",
        "content": processed,
    }));

    // ── Step 5: Discover tools ──────────────────────────────────────────
    let tools = discover_tools(engine);

    log::info!(
        "[chat] Starting agent loop: model={}, history_msgs={}, tools={}",
        model,
        loop_messages.len(),
        tools.len()
    );

    // ── Step 6: Agentic loop ────────────────────────────────────────────
    let mut final_content = String::new();
    let mut total_input_tokens: u64 = 0;
    let mut total_output_tokens: u64 = 0;

    for round in 0..MAX_TOOL_ROUNDS {
        // Check cancellation at the start of each round
        if cancel.is_cancelled() {
            return Err("Request cancelled".to_string());
        }

        // Build request body
        let mut request_body = serde_json::json!({
            "model": model,
            "messages": loop_messages,
        });
        if !tools.is_empty() {
            request_body["tools"] = serde_json::Value::Array(tools.clone());
            request_body["tool_choice"] = serde_json::Value::String("auto".to_string());
        }

        let url = format!("{}/openai/v1/chat/completions", backend_url);
        log::info!(
            "[chat] Round {} — sending inference request ({} messages) to {}",
            round + 1,
            loop_messages.len(),
            url
        );
        log::debug!(
            "[chat] Request body: {}",
            serde_json::to_string_pretty(&request_body).unwrap_or_default()
        );

        // POST to backend with timeout and cancellation support
        let response = tokio::select! {
            _ = cancel.cancelled() => {
                let _ = app.emit("chat:error", ChatErrorEvent {
                    thread_id: thread_id.to_string(),
                    message: "Request cancelled".to_string(),
                    error_type: "cancelled".to_string(),
                    round: Some(round),
                });
                return Err("Request cancelled".to_string());
            }
            result = tokio::time::timeout(
                std::time::Duration::from_secs(INFERENCE_TIMEOUT_SECS),
                client
                    .post(&url)
                    .header("Authorization", format!("Bearer {}", auth_token))
                    .header("Content-Type", "application/json")
                    .json(&request_body)
                    .send()
            ) => {
                match result {
                    Ok(Ok(resp)) => resp,
                    Ok(Err(e)) => {
                        log::error!("[chat] reqwest error detail: {:?}", e);
                        let msg = format!("Network error: {}", e);
                        let _ = app.emit("chat:error", ChatErrorEvent {
                            thread_id: thread_id.to_string(),
                            message: msg.clone(),
                            error_type: "network".to_string(),
                            round: Some(round),
                        });
                        return Err(msg);
                    }
                    Err(_) => {
                        let msg = format!(
                            "Inference request timed out after {}s",
                            INFERENCE_TIMEOUT_SECS
                        );
                        let _ = app.emit("chat:error", ChatErrorEvent {
                            thread_id: thread_id.to_string(),
                            message: msg.clone(),
                            error_type: "timeout".to_string(),
                            round: Some(round),
                        });
                        return Err(msg);
                    }
                }
            }
        };

        // Check HTTP status
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let msg = format!("Backend returned HTTP {}: {}", status, body);
            let _ = app.emit(
                "chat:error",
                ChatErrorEvent {
                    thread_id: thread_id.to_string(),
                    message: msg.clone(),
                    error_type: "inference".to_string(),
                    round: Some(round),
                },
            );
            return Err(msg);
        }

        // Parse the completion response
        let completion: ChatCompletionResponse = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse inference response: {}", e))?;

        // Accumulate token usage
        if let Some(ref usage) = completion.usage {
            total_input_tokens += usage.prompt_tokens;
            total_output_tokens += usage.completion_tokens;
        }

        let choice = completion
            .choices
            .first()
            .ok_or_else(|| "No choices in inference response".to_string())?;

        log::info!(
            "[chat] Round {} — finish_reason={:?}, tool_calls={}",
            round + 1,
            choice.finish_reason,
            choice.message.tool_calls.as_ref().map_or(0, |tc| tc.len())
        );

        // Decide if we have tool calls to execute
        let has_tool_calls = choice.finish_reason.as_deref() == Some("tool_calls")
            && choice
                .message
                .tool_calls
                .as_ref()
                .map_or(false, |tc| !tc.is_empty());

        if has_tool_calls {
            let tool_calls = choice.message.tool_calls.as_ref().unwrap();

            // Append the assistant message with tool_calls to the loop
            loop_messages.push(serde_json::json!({
                "role": "assistant",
                "content": choice.message.content.as_deref().unwrap_or(""),
                "tool_calls": tool_calls,
            }));

            // Execute only the last tool call (matching current TS behaviour);
            // earlier ones get empty placeholder results.
            let latest_idx = tool_calls.len() - 1;

            for (i, tc) in tool_calls.iter().enumerate() {
                if i != latest_idx {
                    loop_messages.push(serde_json::json!({
                        "role": "tool",
                        "tool_call_id": tc.id,
                        "content": "",
                    }));
                    continue;
                }

                let (skill_id, tool_name) = parse_tool_name(&tc.function.name);

                let args_value: serde_json::Value =
                    serde_json::from_str(&tc.function.arguments)
                        .unwrap_or_else(|_| serde_json::Value::Object(serde_json::Map::new()));

                // Emit tool_call event before executing
                let _ = app.emit(
                    "chat:tool_call",
                    ChatToolCallEvent {
                        thread_id: thread_id.to_string(),
                        tool_name: tool_name.clone(),
                        skill_id: skill_id.clone(),
                        args: args_value.clone(),
                        round,
                    },
                );

                // Execute the tool with timeout and cancellation
                let tool_result = tokio::select! {
                    _ = cancel.cancelled() => {
                        return Err("Request cancelled during tool execution".to_string());
                    }
                    result = tokio::time::timeout(
                        std::time::Duration::from_secs(TOOL_TIMEOUT_SECS),
                        engine.call_tool(&skill_id, &tool_name, args_value.clone())
                    ) => {
                        match result {
                            Ok(Ok(r)) => r,
                            Ok(Err(e)) => {
                                let msg = format!("Tool \"{}\" failed: {}", tool_name, e);
                                let _ = app.emit("chat:tool_result", ChatToolResultEvent {
                                    thread_id: thread_id.to_string(),
                                    tool_name: tool_name.clone(),
                                    skill_id: skill_id.clone(),
                                    output: msg.clone(),
                                    success: false,
                                    round,
                                });
                                let _ = app.emit("chat:error", ChatErrorEvent {
                                    thread_id: thread_id.to_string(),
                                    message: msg.clone(),
                                    error_type: "tool_error".to_string(),
                                    round: Some(round),
                                });
                                return Err(msg);
                            }
                            Err(_) => {
                                let msg = format!(
                                    "Tool \"{}\" timed out after {}s",
                                    tool_name, TOOL_TIMEOUT_SECS
                                );
                                let _ = app.emit("chat:error", ChatErrorEvent {
                                    thread_id: thread_id.to_string(),
                                    message: msg.clone(),
                                    error_type: "timeout".to_string(),
                                    round: Some(round),
                                });
                                return Err(msg);
                            }
                        }
                    }
                };

                // Extract text content from the tool result
                let tool_content: String = tool_result
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        crate::runtime::types::ToolContent::Text { text } => {
                            Some(text.as_str())
                        }
                        crate::runtime::types::ToolContent::Json { .. } => None,
                    })
                    .collect::<Vec<&str>>()
                    .join("\n");

                // Check for JSON error pattern (matching TS behaviour)
                let (final_tool_str, final_success) = if !tool_result.is_error {
                    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&tool_content) {
                        if let Some(error_str) =
                            parsed.get("error").and_then(|e| e.as_str())
                        {
                            (format!("Error: {}", error_str), false)
                        } else {
                            (tool_content.clone(), true)
                        }
                    } else {
                        (tool_content.clone(), true)
                    }
                } else {
                    let prefixed = if tool_content.starts_with("Error: ") {
                        tool_content.clone()
                    } else {
                        format!("Error: {}", tool_content)
                    };
                    (prefixed, false)
                };

                // Emit tool_result event
                let _ = app.emit(
                    "chat:tool_result",
                    ChatToolResultEvent {
                        thread_id: thread_id.to_string(),
                        tool_name: tool_name.clone(),
                        skill_id: skill_id.clone(),
                        output: final_tool_str.clone(),
                        success: final_success,
                        round,
                    },
                );

                // Append tool result to loop messages
                loop_messages.push(serde_json::json!({
                    "role": "tool",
                    "tool_call_id": tc.id,
                    "content": final_tool_str,
                }));
            }

            // Continue to the next round
            continue;
        }

        // Non-tool response — the agent loop is done
        final_content = choice.message.content.clone().unwrap_or_default();

        let _ = app.emit(
            "chat:done",
            ChatDoneEvent {
                thread_id: thread_id.to_string(),
                full_response: final_content.clone(),
                rounds_used: round + 1,
                total_input_tokens,
                total_output_tokens,
            },
        );

        return Ok(());
    }

    // Exhausted all rounds — emit whatever we have
    let _ = app.emit(
        "chat:done",
        ChatDoneEvent {
            thread_id: thread_id.to_string(),
            full_response: final_content,
            rounds_used: MAX_TOOL_ROUNDS,
            total_input_tokens,
            total_output_tokens,
        },
    );

    Ok(())
}

// ─── Inner implementation (mobile) ───────────────────────────────────────────

/// Simplified agentic loop for mobile — no tool execution, just a single inference call.
#[cfg(any(target_os = "android", target_os = "ios"))]
async fn chat_send_mobile(
    app: &tauri::AppHandle,
    thread_id: &str,
    user_message: &str,
    model: &str,
    auth_token: &str,
    backend_url: &str,
    history: Vec<ChatMessagePayload>,
    notion_context: Option<String>,
    memory_client: Option<crate::memory::MemoryClientRef>,
    cancel: &CancellationToken,
) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .use_rustls_tls()
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))?;

    // ── Step 1: Load AI context ─────────────────────────────────────────
    let openclaw_context = load_openclaw_context(app);

    // ── Step 2: Recall memory context ───────────────────────────────────
    let memory_context: Option<String> = if let Some(ref mem) = memory_client {
        match mem
            .recall_skill_context("conversations", thread_id, 10)
            .await
        {
            Ok(ctx) => ctx,
            Err(e) => {
                log::warn!("[chat] Memory recall failed: {}", e);
                None
            }
        }
    } else {
        None
    };

    // ── Step 3: Build processed user message ────────────────────────────
    let mut processed = user_message.to_string();

    if !openclaw_context.is_empty() {
        processed = format!("{}\n\nUser message: {}", openclaw_context, processed);
    }

    if let Some(ref mem) = memory_context {
        processed = format!(
            "[MEMORY_CONTEXT]\n{}\n[/MEMORY_CONTEXT]\n\n{}",
            mem, processed
        );
    }

    if let Some(ref notion) = notion_context {
        processed = format!("{}\n\n{}", notion, processed);
    }

    // ── Step 4: Build messages array ─────────────────────────────────────
    let mut messages: Vec<serde_json::Value> = history
        .iter()
        .map(|m| {
            let mut obj = serde_json::json!({
                "role": m.role,
                "content": m.content,
            });
            if let Some(ref tc) = m.tool_calls {
                obj["tool_calls"] = serde_json::to_value(tc).unwrap_or_default();
            }
            if let Some(ref id) = m.tool_call_id {
                obj["tool_call_id"] = serde_json::Value::String(id.clone());
            }
            obj
        })
        .collect();

    messages.push(serde_json::json!({
        "role": "user",
        "content": processed,
    }));

    if cancel.is_cancelled() {
        return Err("Request cancelled".to_string());
    }

    let request_body = serde_json::json!({
        "model": model,
        "messages": messages,
    });

    log::info!(
        "[chat] Mobile inference: model={}, msgs={}",
        model,
        messages.len()
    );

    let response = tokio::select! {
        _ = cancel.cancelled() => {
            let _ = app.emit("chat:error", ChatErrorEvent {
                thread_id: thread_id.to_string(),
                message: "Request cancelled".to_string(),
                error_type: "cancelled".to_string(),
                round: Some(0),
            });
            return Err("Request cancelled".to_string());
        }
        result = tokio::time::timeout(
            std::time::Duration::from_secs(INFERENCE_TIMEOUT_SECS),
            client
                .post(format!("{}/openai/v1/chat/completions", backend_url))
                .header("Authorization", format!("Bearer {}", auth_token))
                .header("Content-Type", "application/json")
                .json(&request_body)
                .send()
        ) => {
            match result {
                Ok(Ok(resp)) => resp,
                Ok(Err(e)) => {
                    let msg = format!("Network error: {}", e);
                    let _ = app.emit("chat:error", ChatErrorEvent {
                        thread_id: thread_id.to_string(),
                        message: msg.clone(),
                        error_type: "network".to_string(),
                        round: Some(0),
                    });
                    return Err(msg);
                }
                Err(_) => {
                    let msg = format!(
                        "Inference request timed out after {}s",
                        INFERENCE_TIMEOUT_SECS
                    );
                    let _ = app.emit("chat:error", ChatErrorEvent {
                        thread_id: thread_id.to_string(),
                        message: msg.clone(),
                        error_type: "timeout".to_string(),
                        round: Some(0),
                    });
                    return Err(msg);
                }
            }
        }
    };

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        let msg = format!("Backend returned HTTP {}: {}", status, body);
        let _ = app.emit(
            "chat:error",
            ChatErrorEvent {
                thread_id: thread_id.to_string(),
                message: msg.clone(),
                error_type: "inference".to_string(),
                round: Some(0),
            },
        );
        return Err(msg);
    }

    let completion: ChatCompletionResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse inference response: {}", e))?;

    let (total_input_tokens, total_output_tokens) = completion
        .usage
        .map(|u| (u.prompt_tokens, u.completion_tokens))
        .unwrap_or((0, 0));

    let choice = completion
        .choices
        .first()
        .ok_or_else(|| "No choices in inference response".to_string())?;

    let full_response = choice.message.content.clone().unwrap_or_default();

    let _ = app.emit(
        "chat:done",
        ChatDoneEvent {
            thread_id: thread_id.to_string(),
            full_response,
            rounds_used: 1,
            total_input_tokens,
            total_output_tokens,
        },
    );

    Ok(())
}
