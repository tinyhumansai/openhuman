//! QjsSkillInstance — manages one QuickJS context per skill.
//!
//! Key differences from V8 version:
//! - QuickJS contexts are Send+Sync with `parallel` feature, so we use regular tokio::spawn (not spawn_blocking)
//! - No V8 creation lock needed (QuickJS contexts are lightweight ~1-2MB)
//! - No stagger delay needed between skill starts
//! - Direct memory limits via `rt.set_memory_limit()`
//! - Uses `ctx.eval::<T, _>(code)` instead of `runtime.execute_script()`
//! - Simplified error handling with rquickjs::Error

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::RwLock;
use tokio::sync::mpsc;

use crate::runtime::cron_scheduler::CronScheduler;
use crate::runtime::skill_registry::SkillRegistry;
use crate::runtime::types::{
    SkillConfig, SkillMessage, SkillSnapshot, SkillStatus, ToolContent, ToolDefinition, ToolResult,
};
use crate::services::tdlib_v8::{qjs_ops, IdbStorage};

/// Dependencies passed to a skill instance for bridge installation.
#[allow(dead_code)]
pub struct BridgeDeps {
    pub cron_scheduler: Arc<CronScheduler>,
    pub skill_registry: Arc<SkillRegistry>,
    pub app_handle: Option<tauri::AppHandle>,
    pub data_dir: PathBuf,
    // NOTE: No v8_creation_lock - QuickJS doesn't need it
}

/// Shared mutable state for a skill instance.
pub struct SkillState {
    pub status: SkillStatus,
    pub tools: Vec<ToolDefinition>,
    pub error: Option<String>,
    pub published_state: HashMap<String, serde_json::Value>,
}

impl Default for SkillState {
    fn default() -> Self {
        Self {
            status: SkillStatus::Pending,
            tools: Vec::new(),
            error: None,
            published_state: HashMap::new(),
        }
    }
}

/// A running skill instance using QuickJS.
pub struct QjsSkillInstance {
    pub config: SkillConfig,
    pub state: Arc<RwLock<SkillState>>,
    pub sender: mpsc::Sender<SkillMessage>,
    pub skill_dir: PathBuf,
    pub data_dir: PathBuf,
}

impl QjsSkillInstance {
    /// Create a new QuickJS skill instance.
    pub fn new(
        config: SkillConfig,
        skill_dir: PathBuf,
        data_dir: PathBuf,
    ) -> (Self, mpsc::Receiver<SkillMessage>) {
        let (tx, rx) = mpsc::channel(64);
        let instance = Self {
            config,
            state: Arc::new(RwLock::new(SkillState::default())),
            sender: tx,
            skill_dir,
            data_dir,
        };
        (instance, rx)
    }

    /// Take a snapshot of the current skill state.
    pub fn snapshot(&self) -> SkillSnapshot {
        let state = self.state.read();
        SkillSnapshot {
            skill_id: self.config.skill_id.clone(),
            name: self.config.name.clone(),
            status: state.status,
            tools: state.tools.clone(),
            error: state.error.clone(),
            state: state.published_state.clone(),
        }
    }

    /// Spawn the skill's execution loop as a tokio task.
    /// Unlike V8 (which needed spawn_blocking), QuickJS contexts are Send.
    pub fn spawn(
        &self,
        mut rx: mpsc::Receiver<SkillMessage>,
        _deps: BridgeDeps,
    ) -> tokio::task::JoinHandle<()> {
        let config = self.config.clone();
        let state = self.state.clone();
        let skill_dir = self.skill_dir.clone();
        let data_dir = self.data_dir.clone();

        tokio::spawn(async move {
            // Update status
            state.write().status = SkillStatus::Initializing;

            // Create storage
            let storage = match IdbStorage::new(&data_dir) {
                Ok(s) => s,
                Err(e) => {
                    let mut s = state.write();
                    s.status = SkillStatus::Error;
                    s.error = Some(format!("Failed to create storage: {e}"));
                    log::error!("[skill:{}] Storage creation failed: {e}", config.skill_id);
                    return;
                }
            };

            // Read the entry point JS file
            let entry_path = skill_dir.join(&config.entry_point);
            let js_source = match tokio::fs::read_to_string(&entry_path).await {
                Ok(src) => src,
                Err(e) => {
                    let mut s = state.write();
                    s.status = SkillStatus::Error;
                    s.error = Some(format!("Failed to read {}: {e}", config.entry_point));
                    log::error!("[skill:{}] Failed to read entry point: {e}", config.skill_id);
                    return;
                }
            };

            // Create QuickJS runtime with memory limits
            let rt = match rquickjs::AsyncRuntime::new() {
                Ok(rt) => rt,
                Err(e) => {
                    let mut s = state.write();
                    s.status = SkillStatus::Error;
                    s.error = Some(format!("Failed to create QuickJS runtime: {e}"));
                    log::error!("[skill:{}] Failed to create QuickJS runtime: {e}", config.skill_id);
                    return;
                }
            };

            // Set memory limit (config.memory_limit is in bytes)
            rt.set_memory_limit(config.memory_limit).await;
            rt.set_max_stack_size(512 * 1024).await; // 512KB stack

            // Create context with full standard library
            let ctx = match rquickjs::AsyncContext::full(&rt).await {
                Ok(ctx) => ctx,
                Err(e) => {
                    let mut s = state.write();
                    s.status = SkillStatus::Error;
                    s.error = Some(format!("Failed to create QuickJS context: {e}"));
                    log::error!("[skill:{}] Failed to create context: {e}", config.skill_id);
                    return;
                }
            };

            // Create shared timer state
            let timer_state = Arc::new(RwLock::new(qjs_ops::TimerState::default()));

            // Create WebSocket state
            let ws_state = Arc::new(RwLock::new(qjs_ops::WebSocketState::default()));

            // Create published skill state (different from SkillState above)
            let published_state = Arc::new(RwLock::new(qjs_ops::SkillState::default()));

            // Register ops and run bootstrap + skill code
            let skill_id = config.skill_id.clone();
            let init_result = ctx.with(|js_ctx| {
                // Register native ops as __ops global
                let skill_context = qjs_ops::SkillContext {
                    skill_id: skill_id.clone(),
                    data_dir: data_dir.clone(),
                };

                if let Err(e) = qjs_ops::register_ops(
                    &js_ctx,
                    storage.clone(),
                    skill_context,
                    published_state.clone(),
                    timer_state.clone(),
                    ws_state.clone(),
                ) {
                    return Err(format!("Failed to register ops: {e}"));
                }

                // Load bootstrap
                let bootstrap_code = include_str!("../services/tdlib_v8/bootstrap.js");
                if let Err(e) = js_ctx.eval::<rquickjs::Value, _>(bootstrap_code) {
                    let detail = format_js_exception(&js_ctx, &e);
                    return Err(format!("Bootstrap failed: {detail}"));
                }

                // Set skill ID
                let bridge_code = format!(
                    r#"globalThis.__skillId = "{}";"#,
                    skill_id.replace('"', r#"\""#)
                );
                if let Err(e) = js_ctx.eval::<rquickjs::Value, _>(bridge_code.as_bytes()) {
                    let detail = format_js_exception(&js_ctx, &e);
                    return Err(format!("Skill init failed: {detail}"));
                }

                // Execute the skill's entry point
                if let Err(e) = js_ctx.eval::<rquickjs::Value, _>(js_source.as_bytes()) {
                    let detail = format_js_exception(&js_ctx, &e);
                    return Err(format!("Skill load failed: {detail}"));
                }

                // Extract tool definitions
                extract_tools(&js_ctx, &state);

                Ok(())
            }).await;

            if let Err(e) = init_result {
                let mut s = state.write();
                s.status = SkillStatus::Error;
                s.error = Some(e.clone());
                log::error!("[skill:{}] {e}", config.skill_id);
                return;
            }

            // Call init() lifecycle
            if let Err(e) = call_lifecycle(&ctx, "init").await {
                let mut s = state.write();
                s.status = SkillStatus::Error;
                s.error = Some(format!("init() failed: {e}"));
                log::error!("[skill:{}] init() failed: {e}", config.skill_id);
                return;
            }

            // Execute pending jobs after init (process promises)
            drive_jobs(&rt).await;

            // Call start() lifecycle
            if let Err(e) = call_lifecycle(&ctx, "start").await {
                let mut s = state.write();
                s.status = SkillStatus::Error;
                s.error = Some(format!("start() failed: {e}"));
                log::error!("[skill:{}] start() failed: {e}", config.skill_id);
                return;
            }

            // Execute pending jobs after start
            drive_jobs(&rt).await;

            // Mark as running
            state.write().status = SkillStatus::Running;
            log::info!("[skill:{}] Running (QuickJS)", config.skill_id);

            // Run the event loop
            run_event_loop(&rt, &ctx, &mut rx, &state, &config.skill_id, &timer_state).await;
        })
    }
}

// ============================================================================
// Event Loop
// ============================================================================

/// The main event loop that drives the QuickJS runtime.
/// This continuously:
/// 1. Polls for ready timers and fires their callbacks
/// 2. Checks for incoming messages (non-blocking)
/// 3. Runs the QuickJS job queue for promises/async ops
/// 4. Sleeps efficiently when idle
async fn run_event_loop(
    rt: &rquickjs::AsyncRuntime,
    ctx: &rquickjs::AsyncContext,
    rx: &mut mpsc::Receiver<SkillMessage>,
    state: &Arc<RwLock<SkillState>>,
    skill_id: &str,
    timer_state: &Arc<RwLock<qjs_ops::TimerState>>,
) {
    // Maximum sleep duration when no timers are pending
    const MAX_IDLE_SLEEP: Duration = Duration::from_millis(100);
    // Minimum sleep to prevent busy-spinning
    const MIN_SLEEP: Duration = Duration::from_millis(1);

    loop {
        // 1. Poll and fire ready timers
        let ready_timers = {
            let (ready, _next) = qjs_ops::poll_timers(timer_state);
            ready
        };

        // Fire timer callbacks in JavaScript
        for timer_id in ready_timers {
            fire_timer_callback(ctx, timer_id).await;
        }

        // 2. Check for incoming messages (non-blocking)
        match rx.try_recv() {
            Ok(msg) => {
                let should_stop = handle_message(ctx, msg, state, skill_id).await;
                if should_stop {
                    break;
                }
            }
            Err(mpsc::error::TryRecvError::Empty) => {
                // No message - that's fine
            }
            Err(mpsc::error::TryRecvError::Disconnected) => {
                // Channel closed, exit
                log::info!("[skill:{}] Message channel disconnected, stopping", skill_id);
                break;
            }
        }

        // 3. Drive QuickJS job queue (process pending promises)
        drive_jobs(rt).await;

        // 4. Calculate sleep duration based on next timer
        let sleep_duration = {
            let (_, next_timer) = qjs_ops::poll_timers(timer_state);
            match next_timer {
                Some(d) if d < MIN_SLEEP => MIN_SLEEP,
                Some(d) if d > MAX_IDLE_SLEEP => MAX_IDLE_SLEEP,
                Some(d) => d,
                None => MAX_IDLE_SLEEP,
            }
        };

        // Sleep efficiently - this yields the thread when no work is needed
        tokio::time::sleep(sleep_duration).await;
    }
}

/// Drive the QuickJS job queue until no more jobs are pending.
async fn drive_jobs(rt: &rquickjs::AsyncRuntime) {
    // idle() runs all pending futures and jobs
    rt.idle().await;
}

/// Fire a timer callback in JavaScript.
async fn fire_timer_callback(ctx: &rquickjs::AsyncContext, timer_id: u32) {
    let code = format!("globalThis.__handleTimer({});", timer_id);
    ctx.with(|js_ctx| {
        if let Err(e) = js_ctx.eval::<rquickjs::Value, _>(code.as_bytes()) {
            log::error!("[timer] Callback for timer {} failed: {}", timer_id, e);
        }
    }).await;
}

/// Handle a single message from the channel.
/// Returns true if the skill should stop.
async fn handle_message(
    ctx: &rquickjs::AsyncContext,
    msg: SkillMessage,
    state: &Arc<RwLock<SkillState>>,
    skill_id: &str,
) -> bool {
    match msg {
        SkillMessage::CallTool { tool_name, arguments, reply } => {
            let result = handle_tool_call(ctx, &tool_name, arguments).await;
            let _ = reply.send(result);
        }
        SkillMessage::ServerEvent { event, data } => {
            let _ = handle_server_event(ctx, &event, data).await;
        }
        SkillMessage::CronTrigger { schedule_id } => {
            let _ = handle_cron_trigger(ctx, &schedule_id).await;
        }
        SkillMessage::Stop { reply } => {
            let _ = call_lifecycle(ctx, "stop").await;
            state.write().status = SkillStatus::Stopped;
            log::info!("[skill:{}] Stopped", skill_id);
            let _ = reply.send(());
            return true; // Signal to stop
        }
        SkillMessage::SetupStart { reply } => {
            let result = handle_js_call(ctx, "onSetupStart", "{}").await;
            let _ = reply.send(result);
        }
        SkillMessage::SetupSubmit { step_id, values, reply } => {
            let args = serde_json::json!({ "stepId": step_id, "values": values });
            let result = handle_js_call(ctx, "onSetupSubmit", &args.to_string()).await;
            let _ = reply.send(result);
        }
        SkillMessage::SetupCancel { reply } => {
            let result = handle_js_void_call(ctx, "onSetupCancel", "{}").await;
            let _ = reply.send(result);
        }
        SkillMessage::ListOptions { reply } => {
            let result = handle_js_call(ctx, "onListOptions", "{}").await;
            let _ = reply.send(result);
        }
        SkillMessage::SetOption { name, value, reply } => {
            let args = serde_json::json!({ "name": name, "value": value });
            let result = handle_js_void_call(ctx, "onSetOption", &args.to_string()).await;
            let _ = reply.send(result);
        }
        SkillMessage::SessionStart { session_id, reply } => {
            let args = serde_json::json!({ "sessionId": session_id });
            let result = handle_js_void_call(ctx, "onSessionStart", &args.to_string()).await;
            let _ = reply.send(result);
        }
        SkillMessage::SessionEnd { session_id, reply } => {
            let args = serde_json::json!({ "sessionId": session_id });
            let result = handle_js_void_call(ctx, "onSessionEnd", &args.to_string()).await;
            let _ = reply.send(result);
        }
        SkillMessage::Tick { reply } => {
            let result = handle_js_void_call(ctx, "onTick", "{}").await;
            let _ = reply.send(result);
        }
        SkillMessage::Rpc { method, params, reply } => {
            let result = match method.as_str() {
                "oauth/complete" => {
                    // Set credential on the oauth bridge, then call onOAuthComplete
                    let set_cred_code = format!(
                        "if (typeof globalThis.oauth !== 'undefined' && globalThis.oauth.__setCredential) {{ globalThis.oauth.__setCredential({}); }}",
                        serde_json::to_string(&params).unwrap_or_else(|_| "null".to_string())
                    );
                    ctx.with(|js_ctx| {
                        let _ = js_ctx.eval::<rquickjs::Value, _>(set_cred_code.as_bytes());
                    }).await;
                    let params_str = serde_json::to_string(&params).unwrap_or_else(|_| "{}".to_string());
                    handle_js_call(ctx, "onOAuthComplete", &params_str).await
                }
                "oauth/revoked" => {
                    // Clear credential on the oauth bridge, then call onOAuthRevoked
                    let clear_code = "if (typeof globalThis.oauth !== 'undefined' && globalThis.oauth.__setCredential) { globalThis.oauth.__setCredential(null); }";
                    ctx.with(|js_ctx| {
                        let _ = js_ctx.eval::<rquickjs::Value, _>(clear_code.as_bytes());
                    }).await;
                    let params_str = serde_json::to_string(&params).unwrap_or_else(|_| "{}".to_string());
                    handle_js_void_call(ctx, "onOAuthRevoked", &params_str).await
                        .map(|()| serde_json::json!({ "ok": true }))
                }
                _ => {
                    let args = serde_json::json!({ "method": method, "params": params });
                    handle_js_call(ctx, "onRpc", &args.to_string()).await
                }
            };
            let _ = reply.send(result);
        }
    }
    false // Don't stop
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Extract a human-readable error message from a QuickJS exception.
/// When rquickjs returns `Error::Exception`, the actual JS error value
/// is stored in the context and retrieved with `Ctx::catch()`.
fn format_js_exception(js_ctx: &rquickjs::Ctx<'_>, err: &rquickjs::Error) -> String {
    if !err.is_exception() {
        return format!("{err}");
    }

    let exception = js_ctx.catch();

    // Try to get .message and .stack from the exception object
    if let Some(obj) = exception.as_object() {
        let message: String = obj
            .get::<_, String>("message")
            .unwrap_or_default();
        let stack: String = obj
            .get::<_, String>("stack")
            .unwrap_or_default();

        if !message.is_empty() {
            if !stack.is_empty() {
                return format!("{message}\n{stack}");
            }
            return message;
        }
    }

    // Fallback: try to stringify the exception value
    if let Ok(s) = exception.get::<String>() {
        return s;
    }

    format!("{err}")
}

/// Call a lifecycle function on the skill object.
/// Looks for the skill at globalThis.__skill.default first, then falls back to globalThis.
async fn call_lifecycle(ctx: &rquickjs::AsyncContext, name: &str) -> Result<(), String> {
    let name = name.to_string();
    ctx.with(|js_ctx| {
        let code = format!(
            r#"(function() {{
                var skill = globalThis.__skill && globalThis.__skill.default
                    ? globalThis.__skill.default
                    : (globalThis.__skill || globalThis);
                if (typeof skill.{name} === 'function') {{
                    skill.{name}();
                }} else if (typeof globalThis.{name} === 'function') {{
                    globalThis.{name}();
                }}
            }})()"#
        );
        js_ctx.eval::<rquickjs::Value, _>(code.as_bytes())
            .map_err(|e| {
                let detail = format_js_exception(&js_ctx, &e);
                format!("{name}() failed: {detail}")
            })?;
        Ok(())
    }).await
}

/// Extract tool definitions from the skill.
fn extract_tools(js_ctx: &rquickjs::Ctx<'_>, state: &Arc<RwLock<SkillState>>) {
    let code = r#"
        (function() {
            var skill = globalThis.__skill && globalThis.__skill.default
                ? globalThis.__skill.default
                : (globalThis.__skill || null);
            var tools = (skill && skill.tools) || globalThis.tools || [];
            return JSON.stringify(tools.map(function(t) {
                return {
                    name: t.name || "",
                    description: t.description || "",
                    input_schema: t.inputSchema || t.input_schema || {}
                };
            }));
        })()
    "#;

    // eval with String type hint tells rquickjs to convert the result to a Rust String
    match js_ctx.eval::<String, _>(code) {
        Ok(json_str) => {
            match serde_json::from_str::<Vec<ToolDefinition>>(&json_str) {
                Ok(tools) => {
                    state.write().tools = tools;
                }
                Err(e) => {
                    log::warn!("[tools] Failed to parse tools JSON: {e}");
                }
            }
        }
        Err(e) => {
            log::warn!("[tools] Failed to extract tools: {e}");
        }
    }
}

/// Handle a tool call.
async fn handle_tool_call(
    ctx: &rquickjs::AsyncContext,
    tool_name: &str,
    arguments: serde_json::Value,
) -> Result<ToolResult, String> {
    let args_str = serde_json::to_string(&arguments)
        .map_err(|e| format!("Failed to serialize args: {e}"))?;
    let tool_name = tool_name.to_string();

    let result_text = ctx.with(|js_ctx| {
        let code = format!(
            r#"(function() {{
                var skill = globalThis.__skill && globalThis.__skill.default
                    ? globalThis.__skill.default
                    : (globalThis.__skill || null);
                var tools = (skill && skill.tools) || globalThis.tools || [];
                for (var i = 0; i < tools.length; i++) {{
                    if (tools[i].name === "{}") {{
                        var args = {};
                        var result = tools[i].execute(args);
                        if (result && typeof result === 'object') {{
                            return JSON.stringify(result);
                        }}
                        return String(result);
                    }}
                }}
                throw new Error("Tool '{}' not found");
            }})()"#,
            tool_name.replace('"', r#"\""#),
            args_str,
            tool_name.replace('"', r#"\""#),
        );

        match js_ctx.eval::<String, _>(code.as_bytes()) {
            Ok(s) => Ok(s),
            Err(e) => {
                let detail = format_js_exception(&js_ctx, &e);
                Err(format!("Tool execution failed: {detail}"))
            }
        }
    }).await?;

    Ok(ToolResult {
        content: vec![ToolContent::Text { text: result_text }],
        is_error: false,
    })
}

/// Handle a server event.
async fn handle_server_event(
    ctx: &rquickjs::AsyncContext,
    event: &str,
    data: serde_json::Value,
) -> Result<(), String> {
    let data_str = serde_json::to_string(&data).unwrap_or_else(|_| "null".to_string());
    let event = event.to_string();

    ctx.with(|js_ctx| {
        let code = format!(
            r#"(function() {{
                var skill = globalThis.__skill && globalThis.__skill.default
                    ? globalThis.__skill.default
                    : (globalThis.__skill || globalThis);
                if (typeof skill.onServerEvent === 'function') {{
                    skill.onServerEvent("{}", {});
                }} else if (typeof globalThis.onServerEvent === 'function') {{
                    globalThis.onServerEvent("{}", {});
                }}
            }})()"#,
            event.replace('"', r#"\""#),
            data_str,
            event.replace('"', r#"\""#),
            data_str,
        );

        js_ctx.eval::<rquickjs::Value, _>(code.as_bytes())
            .map_err(|e| format!("Event handler failed: {e}"))?;
        Ok(())
    }).await
}

/// Handle a cron trigger.
async fn handle_cron_trigger(ctx: &rquickjs::AsyncContext, schedule_id: &str) -> Result<(), String> {
    let schedule_id = schedule_id.to_string();
    ctx.with(|js_ctx| {
        let code = format!(
            r#"(function() {{
                var skill = globalThis.__skill && globalThis.__skill.default
                    ? globalThis.__skill.default
                    : (globalThis.__skill || globalThis);
                if (typeof skill.onCronTrigger === 'function') {{
                    skill.onCronTrigger("{}");
                }} else if (typeof globalThis.onCronTrigger === 'function') {{
                    globalThis.onCronTrigger("{}");
                }}
            }})()"#,
            schedule_id.replace('"', r#"\""#),
            schedule_id.replace('"', r#"\""#),
        );
        js_ctx.eval::<rquickjs::Value, _>(code.as_bytes())
            .map_err(|e| format!("Cron trigger failed: {e}"))
            .map(|_| ())
    }).await
}

/// Call a JS function on the skill object that returns a JSON value.
async fn handle_js_call(
    ctx: &rquickjs::AsyncContext,
    fn_name: &str,
    args_json: &str,
) -> Result<serde_json::Value, String> {
    let fn_name = fn_name.to_string();
    let args_json = args_json.to_string();

    let result_text = ctx.with(|js_ctx| {
        let code = format!(
            r#"(function() {{
                var skill = globalThis.__skill && globalThis.__skill.default
                    ? globalThis.__skill.default
                    : (globalThis.__skill || globalThis);
                var fn = skill.{fn_name} || globalThis.{fn_name};
                if (typeof fn === 'function') {{
                    var args = {args_json};
                    var result = fn.call(skill, args);
                    return JSON.stringify(result);
                }}
                return "null";
            }})()"#
        );

        match js_ctx.eval::<String, _>(code.as_bytes()) {
            Ok(s) => Ok(s),
            Err(e) => {
                let detail = format_js_exception(&js_ctx, &e);
                Err(format!("{fn_name}() failed: {detail}"))
            }
        }
    }).await?;

    serde_json::from_str(&result_text)
        .map_err(|e| format!("{fn_name}() returned invalid JSON: {e}"))
}

/// Call a JS function on the skill object that returns void.
async fn handle_js_void_call(
    ctx: &rquickjs::AsyncContext,
    fn_name: &str,
    args_json: &str,
) -> Result<(), String> {
    let fn_name = fn_name.to_string();
    let args_json = args_json.to_string();

    ctx.with(|js_ctx| {
        let code = format!(
            r#"(function() {{
                var skill = globalThis.__skill && globalThis.__skill.default
                    ? globalThis.__skill.default
                    : (globalThis.__skill || globalThis);
                var fn = skill.{fn_name} || globalThis.{fn_name};
                if (typeof fn === 'function') {{
                    var args = {args_json};
                    fn.call(skill, args);
                }}
            }})()"#
        );

        js_ctx.eval::<rquickjs::Value, _>(code.as_bytes())
            .map_err(|e| format!("{fn_name}() failed: {e}"))
            .map(|_| ())
    }).await
}
