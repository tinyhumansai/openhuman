//! V8SkillInstance — manages one V8 context per skill.
//!
//! Each skill runs on its own dedicated thread (V8's JsRuntime is not Send)
//! with:
//! - A scoped SQLite database
//! - Bridge globals (db, store, net, platform, console)
//! - An async event loop that drives timers, promises, and handles messages
//! - Lifecycle hooks: init() -> start() -> [event loop] -> stop()

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use deno_core::{v8, JsRuntime, PollEventLoopOptions, RuntimeOptions};
use parking_lot::RwLock;
use tokio::sync::mpsc;

use crate::runtime::cron_scheduler::CronScheduler;
use crate::runtime::skill_registry::SkillRegistry;
use crate::runtime::types::{
    SkillConfig, SkillMessage, SkillSnapshot, SkillStatus, ToolContent, ToolDefinition, ToolResult,
};
use crate::services::tdlib_v8::{ops, IdbStorage};

/// Dependencies passed to a skill instance for bridge installation.
/// Currently not all fields are used, but they're kept for future feature parity.
#[allow(dead_code)]
pub struct BridgeDeps {
    pub cron_scheduler: Arc<CronScheduler>,
    pub skill_registry: Arc<SkillRegistry>,
    pub app_handle: Option<tauri::AppHandle>,
    pub data_dir: PathBuf,
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

/// A running skill instance using V8.
pub struct V8SkillInstance {
    pub config: SkillConfig,
    pub state: Arc<RwLock<SkillState>>,
    pub sender: mpsc::Sender<SkillMessage>,
    pub skill_dir: PathBuf,
    pub data_dir: PathBuf,
}

impl V8SkillInstance {
    /// Create a new V8 skill instance.
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

    /// Spawn the skill's execution loop in a dedicated thread.
    /// Returns a JoinHandle wrapped in a tokio task for compatibility.
    pub fn spawn(
        &self,
        mut rx: mpsc::Receiver<SkillMessage>,
        _deps: BridgeDeps,
    ) -> tokio::task::JoinHandle<()> {
        let config = self.config.clone();
        let state = self.state.clone();
        let skill_dir = self.skill_dir.clone();
        let data_dir = self.data_dir.clone();

        // Use std::thread::spawn since JsRuntime is not Send
        // Wrap in tokio task for API compatibility
        tokio::task::spawn_blocking(move || {
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

            // Read the entry point JS file synchronously
            let entry_path = skill_dir.join(&config.entry_point);
            let js_source = match std::fs::read_to_string(&entry_path) {
                Ok(src) => src,
                Err(e) => {
                    let mut s = state.write();
                    s.status = SkillStatus::Error;
                    s.error = Some(format!("Failed to read {}: {e}", config.entry_point));
                    log::error!("[skill:{}] Failed to read entry point: {e}", config.skill_id);
                    return;
                }
            };

            // Create V8 runtime
            let extension = ops::build_extension(storage.clone());
            let mut runtime = JsRuntime::new(RuntimeOptions {
                extensions: vec![extension],
                ..Default::default()
            });

            // Set skill context in op state
            {
                let op_state = runtime.op_state();
                let mut state_ref = op_state.borrow_mut();
                ops::init_state_with_data_dir(
                    &mut state_ref,
                    storage,
                    config.skill_id.clone(),
                    data_dir.clone(),
                    state.clone(),
                );
            }

            // Load bootstrap
            let bootstrap_code = include_str!("../services/tdlib_v8/bootstrap.js");
            if let Err(e) = runtime.execute_script("<bootstrap>", bootstrap_code.to_string()) {
                let mut s = state.write();
                s.status = SkillStatus::Error;
                s.error = Some(format!("Bootstrap failed: {e}"));
                log::error!("[skill:{}] Bootstrap failed: {e}", config.skill_id);
                return;
            }

            // Install skill-specific bridges
            let skill_id = config.skill_id.clone();
            let bridge_code = format!(
                r#"globalThis.__skillId = "{}";"#,
                skill_id.replace('"', r#"\""#)
            );

            if let Err(e) = runtime.execute_script("<skill-init>", bridge_code) {
                let mut s = state.write();
                s.status = SkillStatus::Error;
                s.error = Some(format!("Skill init failed: {e}"));
                return;
            }

            // Execute the skill's entry point
            // Use a static string for the filename
            let filename: &'static str = Box::leak(format!("<skill:{}>", config.skill_id).into_boxed_str());
            if let Err(e) = runtime.execute_script(filename, js_source) {
                let mut s = state.write();
                s.status = SkillStatus::Error;
                s.error = Some(format!("Skill load failed: {e}"));
                log::error!("[skill:{}] Load failed: {e}", config.skill_id);
                return;
            }

            // Extract tool definitions
            extract_tools(&mut runtime, &state);

            // Create a tokio runtime for this thread FIRST - all async ops must run inside it
            // This is critical because deno_unsync requires CurrentThread runtime for async ops
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("Failed to create tokio runtime");

            // Run the entire lifecycle inside the CurrentThread runtime
            rt.block_on(async {
                // Call init()
                if let Err(e) = call_lifecycle_fn_async(&mut runtime, "init").await {
                    let mut s = state.write();
                    s.status = SkillStatus::Error;
                    s.error = Some(format!("init() failed: {e}"));
                    log::error!("[skill:{}] init() failed: {e}", config.skill_id);
                    return;
                }

                // Call start()
                if let Err(e) = call_lifecycle_fn_async(&mut runtime, "start").await {
                    let mut s = state.write();
                    s.status = SkillStatus::Error;
                    s.error = Some(format!("start() failed: {e}"));
                    log::error!("[skill:{}] start() failed: {e}", config.skill_id);
                    return;
                }

                // Mark as running
                state.write().status = SkillStatus::Running;
                log::info!("[skill:{}] Running (V8)", config.skill_id);

                // Run the event loop
                run_event_loop(&mut runtime, &mut rx, &state, &config.skill_id).await;
            });
        })
    }
}

// ============================================================================
// Event Loop
// ============================================================================

/// The main event loop that drives the V8 runtime.
/// This continuously:
/// 1. Polls for ready timers and fires their callbacks
/// 2. Checks for incoming messages (non-blocking)
/// 3. Runs the V8 event loop for promises/async ops
/// 4. Sleeps efficiently when idle
async fn run_event_loop(
    runtime: &mut JsRuntime,
    rx: &mut mpsc::Receiver<SkillMessage>,
    state: &Arc<RwLock<SkillState>>,
    skill_id: &str,
) {
    // Maximum sleep duration when no timers are pending
    const MAX_IDLE_SLEEP: Duration = Duration::from_millis(100);
    // Minimum sleep to prevent busy-spinning
    const MIN_SLEEP: Duration = Duration::from_millis(1);

    loop {
        // 1. Poll and fire ready timers
        let ready_timers = {
            let op_state = runtime.op_state();
            let mut state_ref = op_state.borrow_mut();
            let (ready, _next) = ops::poll_timers(&mut state_ref);
            ready
        };

        // Fire timer callbacks in JavaScript
        for timer_id in ready_timers {
            fire_timer_callback(runtime, timer_id);
        }

        // 2. Check for incoming messages (non-blocking)
        match rx.try_recv() {
            Ok(msg) => {
                let should_stop = handle_message(runtime, msg, state, skill_id).await;
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

        // 3. Run the V8 event loop (processes promises, async ops, etc.)
        // Use poll mode - returns immediately if nothing to do
        let poll_result = runtime
            .run_event_loop(PollEventLoopOptions {
                wait_for_inspector: false,
                pump_v8_message_loop: true,
            })
            .await;

        if let Err(e) = poll_result {
            log::error!("[skill:{}] Event loop error: {}", skill_id, e);
            // Don't break - try to continue
        }

        // 4. Calculate sleep duration based on next timer
        let sleep_duration = {
            let op_state = runtime.op_state();
            let mut state_ref = op_state.borrow_mut();
            let (_, next_timer) = ops::poll_timers(&mut state_ref);
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

/// Fire a timer callback in JavaScript.
fn fire_timer_callback(runtime: &mut JsRuntime, timer_id: u32) {
    let code = format!("globalThis.__handleTimer({});", timer_id);
    if let Err(e) = runtime.execute_script("<timer-callback>", code) {
        log::error!("[timer] Callback for timer {} failed: {}", timer_id, e);
    }
}

/// Handle a single message from the channel.
/// Returns true if the skill should stop.
async fn handle_message(
    runtime: &mut JsRuntime,
    msg: SkillMessage,
    state: &Arc<RwLock<SkillState>>,
    skill_id: &str,
) -> bool {
    match msg {
        SkillMessage::CallTool {
            tool_name,
            arguments,
            reply,
        } => {
            let result = handle_tool_call_sync(runtime, &tool_name, arguments);
            let _ = reply.send(result);
        }
        SkillMessage::ServerEvent { event, data } => {
            let _ = handle_server_event_sync(runtime, &event, data);
        }
        SkillMessage::CronTrigger { schedule_id } => {
            let _ = handle_cron_trigger_sync(runtime, &schedule_id);
        }
        SkillMessage::Stop { reply } => {
            let _ = call_lifecycle_fn_sync(runtime, "stop");
            state.write().status = SkillStatus::Stopped;
            log::info!("[skill:{}] Stopped", skill_id);
            let _ = reply.send(());
            return true; // Signal to stop
        }
        SkillMessage::SetupStart { reply } => {
            let result = handle_js_call_sync(runtime, "onSetupStart", "{}");
            let _ = reply.send(result);
        }
        SkillMessage::SetupSubmit {
            step_id,
            values,
            reply,
        } => {
            let args = serde_json::json!({
                "stepId": step_id,
                "values": values,
            });
            let result = handle_js_call_sync(runtime, "onSetupSubmit", &args.to_string());
            let _ = reply.send(result);
        }
        SkillMessage::SetupCancel { reply } => {
            let result = handle_js_void_call_sync(runtime, "onSetupCancel", "{}");
            let _ = reply.send(result);
        }
        SkillMessage::ListOptions { reply } => {
            let result = handle_js_call_sync(runtime, "onListOptions", "{}");
            let _ = reply.send(result);
        }
        SkillMessage::SetOption { name, value, reply } => {
            let args = serde_json::json!({
                "name": name,
                "value": value,
            });
            let result = handle_js_void_call_sync(runtime, "onSetOption", &args.to_string());
            let _ = reply.send(result);
        }
        SkillMessage::SessionStart { session_id, reply } => {
            let args = serde_json::json!({ "sessionId": session_id });
            let result = handle_js_void_call_sync(runtime, "onSessionStart", &args.to_string());
            let _ = reply.send(result);
        }
        SkillMessage::SessionEnd { session_id, reply } => {
            let args = serde_json::json!({ "sessionId": session_id });
            let result = handle_js_void_call_sync(runtime, "onSessionEnd", &args.to_string());
            let _ = reply.send(result);
        }
        SkillMessage::Tick { reply } => {
            let result = handle_js_void_call_sync(runtime, "onTick", "{}");
            let _ = reply.send(result);
        }
        SkillMessage::Rpc {
            method,
            params,
            reply,
        } => {
            let args = serde_json::json!({
                "method": method,
                "params": params,
            });
            let result = handle_js_call_sync(runtime, "onRpc", &args.to_string());
            let _ = reply.send(result);
        }
    }
    false // Don't stop
}

/// Extract tool definitions from skill.tools (supports both globalThis.__skill.default and globalThis.tools).
fn extract_tools(runtime: &mut JsRuntime, state: &Arc<RwLock<SkillState>>) {
    let code = r#"
        (function() {
            // Try to get skill from bundled export first, then fall back to globalThis.tools
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

    if let Ok(result) = runtime.execute_script("<extract-tools>", code.to_string()) {
        let scope = &mut runtime.handle_scope();
        let local = v8::Local::new(scope, result);

        if let Some(s) = local.to_string(scope) {
            let json_str = s.to_rust_string_lossy(scope);
            if let Ok(tools) = serde_json::from_str::<Vec<ToolDefinition>>(&json_str) {
                state.write().tools = tools;
            }
        }
    }
}

/// Call a lifecycle function on the skill object asynchronously.
/// This version runs inside the skill's CurrentThread runtime - no nested runtime creation.
/// Looks for the skill at globalThis.__skill.default first, then falls back to globalThis.
///
/// Note: This does NOT wait for the event loop to complete, because lifecycle functions
/// may start async operations (like update loops) that run indefinitely. The main event
/// loop will process pending async work after init/start complete.
async fn call_lifecycle_fn_async(runtime: &mut JsRuntime, name: &str) -> Result<(), String> {
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

    runtime
        .execute_script("<lifecycle>", code)
        .map_err(|e| format!("{name}() failed: {e}"))?;

    // Don't wait for event loop here - the main event loop will handle pending async work.
    // This is important because lifecycle functions may start long-running async operations
    // (like TDLib update loops) that would block init/start from completing.

    Ok(())
}

/// Call a lifecycle function on the skill object synchronously.
/// WARNING: This creates its own tokio runtime - only use when NOT inside an async context.
/// Looks for the skill at globalThis.__skill.default first, then falls back to globalThis.
#[allow(dead_code)]
fn call_lifecycle_fn_sync(runtime: &mut JsRuntime, name: &str) -> Result<(), String> {
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

    runtime
        .execute_script("<lifecycle>", code)
        .map_err(|e| format!("{name}() failed: {e}"))?;

    // Run event loop synchronously to handle any pending ops
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("Failed to create runtime: {e}"))?;

    rt.block_on(async {
        runtime
            .run_event_loop(PollEventLoopOptions::default())
            .await
            .map_err(|e| format!("Event loop error: {e}"))
    })?;

    Ok(())
}

/// Handle a tool call synchronously (no async ops waited).
/// This is used when we just need to invoke the tool and get immediate result.
fn handle_tool_call_sync(
    runtime: &mut JsRuntime,
    tool_name: &str,
    arguments: serde_json::Value,
) -> Result<ToolResult, String> {
    let args_str =
        serde_json::to_string(&arguments).map_err(|e| format!("Failed to serialize args: {e}"))?;

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

    let result = runtime
        .execute_script("<tool-call>", code)
        .map_err(|e| format!("Tool execution failed: {e}"))?;

    // Note: We don't run the event loop here because we're already inside
    // the skill's event loop. Pending async ops will be handled by the main loop.

    let scope = &mut runtime.handle_scope();
    let local = v8::Local::new(scope, result);

    let result_text = if let Some(s) = local.to_string(scope) {
        s.to_rust_string_lossy(scope)
    } else {
        "null".to_string()
    };

    Ok(ToolResult {
        content: vec![ToolContent::Text { text: result_text }],
        is_error: false,
    })
}

/// Handle a server event synchronously.
fn handle_server_event_sync(
    runtime: &mut JsRuntime,
    event: &str,
    data: serde_json::Value,
) -> Result<(), String> {
    let data_str = serde_json::to_string(&data).unwrap_or_else(|_| "null".to_string());

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

    runtime
        .execute_script("<server-event>", code)
        .map_err(|e| format!("Event handler failed: {e}"))?;

    Ok(())
}

/// Handle a cron trigger synchronously.
fn handle_cron_trigger_sync(runtime: &mut JsRuntime, schedule_id: &str) -> Result<(), String> {
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

    runtime
        .execute_script("<cron-trigger>", code)
        .map_err(|e| format!("Cron trigger failed: {e}"))?;

    Ok(())
}

/// Call a JS function on the skill object that returns a JSON value synchronously.
fn handle_js_call_sync(
    runtime: &mut JsRuntime,
    fn_name: &str,
    args_json: &str,
) -> Result<serde_json::Value, String> {
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

    let result = runtime
        .execute_script("<js-call>", code)
        .map_err(|e| format!("{fn_name}() failed: {e}"))?;

    let scope = &mut runtime.handle_scope();
    let local = v8::Local::new(scope, result);

    let result_text = if let Some(s) = local.to_string(scope) {
        s.to_rust_string_lossy(scope)
    } else {
        "null".to_string()
    };

    serde_json::from_str(&result_text)
        .map_err(|e| format!("{fn_name}() returned invalid JSON: {e}"))
}

/// Call a JS function on the skill object that returns void synchronously.
fn handle_js_void_call_sync(
    runtime: &mut JsRuntime,
    fn_name: &str,
    args_json: &str,
) -> Result<(), String> {
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

    runtime
        .execute_script("<js-void-call>", code)
        .map_err(|e| format!("{fn_name}() failed: {e}"))?;

    Ok(())
}
