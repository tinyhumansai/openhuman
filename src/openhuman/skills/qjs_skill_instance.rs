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

use crate::openhuman::memory::MemoryState;
use crate::openhuman::skills::cron_scheduler::CronScheduler;
use crate::openhuman::skills::quickjs_libs::{qjs_ops, IdbStorage};
use crate::openhuman::skills::skill_registry::SkillRegistry;
use crate::openhuman::skills::types::{
    SkillConfig, SkillMessage, SkillSnapshot, SkillStatus, ToolContent, ToolDefinition, ToolResult,
};
use tauri::Manager;

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
            let storage: IdbStorage = match IdbStorage::new(&data_dir) {
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
                    log::error!(
                        "[skill:{}] Failed to read entry point: {e}",
                        config.skill_id
                    );
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
                    log::error!(
                        "[skill:{}] Failed to create QuickJS runtime: {e}",
                        config.skill_id
                    );
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
            let init_result = ctx
                .with(|js_ctx| {
                    // Register native ops as __ops global
                    let skill_context = qjs_ops::SkillContext {
                        skill_id: skill_id.clone(),
                        data_dir: data_dir.clone(),
                        app_handle: _deps.app_handle.clone(),
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
                    let bootstrap_code = include_str!("quickjs_libs/bootstrap.js");
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
                })
                .await;

            if let Err(e) = init_result {
                let mut s = state.write();
                s.status = SkillStatus::Error;
                s.error = Some(e.clone());
                log::error!("[skill:{}] {e}", config.skill_id);
                return;
            }

            restore_oauth_credential(&ctx, &config.skill_id).await;

            // Call init() lifecycle
            if let Err(e) = call_lifecycle(&rt, &ctx, "init").await {
                let mut s = state.write();
                s.status = SkillStatus::Error;
                s.error = Some(format!("init() failed: {e}"));
                log::error!("[skill:{}] init() failed: {e}", config.skill_id);
                return;
            }

            // Execute pending jobs after init (process promises)
            drive_jobs(&rt).await;

            // Call start() lifecycle
            if let Err(e) = call_lifecycle(&rt, &ctx, "start").await {
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

            // Immediate ping to verify the connection is healthy
            match handle_js_call(&rt, &ctx, "onPing", "{}").await {
                Ok(value) => {
                    log::info!("[skill:{}] Initial ping result: {}", config.skill_id, value);
                }
                Err(e) => {
                    log::warn!("[skill:{}] Initial ping failed: {}", config.skill_id, e);
                }
            }
            drive_jobs(&rt).await;

            // Run the event loop
            run_event_loop(
                &rt,
                &ctx,
                &mut rx,
                &state,
                &config.skill_id,
                &timer_state,
                &published_state,
                _deps.app_handle.as_ref(),
            )
            .await;
        })
    }
}

// ============================================================================
// Event Loop
// ============================================================================

/// Pending async tool call that is being driven by the event loop.
struct PendingToolCall {
    reply: tokio::sync::oneshot::Sender<Result<ToolResult, String>>,
    deadline: tokio::time::Instant,
}

/// The main event loop that drives the QuickJS runtime.
/// This continuously:
/// 1. Polls for ready timers and fires their callbacks
/// 2. Checks for incoming messages (non-blocking)
/// 3. Runs the QuickJS job queue for promises/async ops
/// 4. Checks if a pending async tool call has completed
/// 5. Syncs published state from ops → instance and emits Tauri events
/// 6. Sleeps efficiently when idle
async fn run_event_loop(
    rt: &rquickjs::AsyncRuntime,
    ctx: &rquickjs::AsyncContext,
    rx: &mut mpsc::Receiver<SkillMessage>,
    state: &Arc<RwLock<SkillState>>,
    skill_id: &str,
    timer_state: &Arc<RwLock<qjs_ops::TimerState>>,
    ops_state: &Arc<RwLock<qjs_ops::SkillState>>,
    app_handle: Option<&tauri::AppHandle>,
) {
    // Maximum sleep duration when no timers are pending
    const MAX_IDLE_SLEEP: Duration = Duration::from_millis(100);
    // Minimum sleep to prevent busy-spinning
    const MIN_SLEEP: Duration = Duration::from_millis(1);
    // Faster polling when waiting for an async tool call
    const TOOL_POLL_SLEEP: Duration = Duration::from_millis(5);

    // Tracks an in-flight async tool call whose Promise hasn't resolved yet.
    let mut pending_tool: Option<PendingToolCall> = None;

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

        // 2. Check for incoming messages (non-blocking).
        //    While an async tool call is in flight, still process other
        //    messages (events, pings, etc.) but queue any new CallTool.
        match rx.try_recv() {
            Ok(msg) => {
                let should_stop = handle_message(
                    rt,
                    ctx,
                    msg,
                    state,
                    skill_id,
                    &mut pending_tool,
                    app_handle,
                    ops_state,
                )
                .await;
                if should_stop {
                    break;
                }
            }
            Err(mpsc::error::TryRecvError::Empty) => {
                // No message - that's fine
            }
            Err(mpsc::error::TryRecvError::Disconnected) => {
                // Channel closed, exit
                log::info!(
                    "[skill:{}] Message channel disconnected, stopping",
                    skill_id
                );
                break;
            }
        }

        // 3. Drive QuickJS job queue (process pending promises)
        drive_jobs(rt).await;

        // 4. Check if pending async tool call has completed
        if pending_tool.is_some() {
            let done = ctx
                .with(|js_ctx| {
                    js_ctx
                        .eval::<bool, _>(b"globalThis.__pendingToolDone === true")
                        .unwrap_or(false)
                })
                .await;

            if done {
                // Read the resolved value and send it back
                let result = read_pending_tool_result(ctx).await;
                if let Some(ptc) = pending_tool.take() {
                    let _ = ptc.reply.send(result);
                }
            } else if let Some(ref ptc) = pending_tool {
                if tokio::time::Instant::now() >= ptc.deadline {
                    log::error!("[skill:{}] Async tool call timed out", skill_id);
                    if let Some(ptc) = pending_tool.take() {
                        let _ = ptc
                            .reply
                            .send(Err("Tool async execution timed out".to_string()));
                    }
                }
            }
        }

        // 5. Sync ops-level published state → instance published_state + emit event
        {
            let mut ops = ops_state.write();
            if ops.dirty {
                ops.dirty = false;
                // Convert serde_json::Map → HashMap for the instance snapshot
                let new_map: HashMap<String, serde_json::Value> = ops
                    .data
                    .iter()
                    .map(|(k, v): (&String, &serde_json::Value)| (k.clone(), v.clone()))
                    .collect();
                state.write().published_state = new_map.clone();

                // Emit Tauri event to the main window so the frontend receives it (Tauri 2: target explicitly)
                if let Some(handle) = app_handle {
                    use tauri::Emitter;
                    let payload = serde_json::json!({
                        "skillId": skill_id,
                        "state": new_map,
                    });
                    if let Err(e) = handle.emit_to("main", "skill-state-changed", payload) {
                        log::warn!(
                            "[skill:{}] Failed to emit skill-state-changed to main: {}",
                            skill_id,
                            e
                        );
                    }
                }
            }
        }

        // 6. Calculate sleep duration based on next timer and pending tool call
        let sleep_duration = if pending_tool.is_some() {
            // Poll faster while waiting for an async tool result
            TOOL_POLL_SLEEP
        } else {
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
    })
    .await;
}

/// Handle a single message from the channel.
/// Returns true if the skill should stop.
async fn handle_message(
    rt: &rquickjs::AsyncRuntime,
    ctx: &rquickjs::AsyncContext,
    msg: SkillMessage,
    state: &Arc<RwLock<SkillState>>,
    skill_id: &str,
    pending_tool: &mut Option<PendingToolCall>,
    app_handle: Option<&tauri::AppHandle>,
    ops_state: &Arc<RwLock<qjs_ops::SkillState>>,
) -> bool {
    match msg {
        SkillMessage::CallTool {
            tool_name,
            arguments,
            reply,
        } => {
            // Lazy-load persisted OAuth credential before calling the tool
            restore_oauth_credential(ctx, skill_id).await;

            // Start the async tool execution. The JS code stores the result
            // in globals when done. The main event loop checks for completion.
            match start_async_tool_call(ctx, &tool_name, arguments).await {
                Ok(Some(sync_result)) => {
                    // Tool returned synchronously (non-Promise)
                    let _ = reply.send(Ok(sync_result));
                }
                Ok(None) => {
                    // Tool returned a Promise — event loop will drive it
                    *pending_tool = Some(PendingToolCall {
                        reply,
                        deadline: tokio::time::Instant::now() + Duration::from_secs(120),
                    });
                }
                Err(e) => {
                    let _ = reply.send(Err(e));
                }
            }
        }
        SkillMessage::ServerEvent { event, data } => {
            let _ = handle_server_event(rt, ctx, &event, data).await;
        }
        SkillMessage::CronTrigger { schedule_id } => {
            let _ = handle_cron_trigger(rt, ctx, &schedule_id).await;
        }
        SkillMessage::Stop { reply } => {
            let _ = call_lifecycle(rt, ctx, "stop").await;

            // Clear OAuth credential from memory and mark as disconnected in store
            let clear_code = r#"(function() {
                if (typeof globalThis.oauth !== 'undefined' && globalThis.oauth.__setCredential) {
                    globalThis.oauth.__setCredential(null);
                }
                if (typeof globalThis.state !== 'undefined' && globalThis.state.set) {
                    globalThis.state.set('__oauth_credential', '');
                }
            })()"#;
            ctx.with(|js_ctx| {
                let _ = js_ctx.eval::<rquickjs::Value, _>(clear_code.as_bytes());
            })
            .await;
            state.write().status = SkillStatus::Stopped;
            log::info!("[skill:{}] Stopped (OAuth credential cleared)", skill_id);
            let _ = reply.send(());

            return true; // Signal to stop
        }
        SkillMessage::SetupStart { reply } => {
            let result = handle_js_call(rt, ctx, "onSetupStart", "{}").await;
            let _ = reply.send(result);
        }
        SkillMessage::SetupSubmit {
            step_id,
            values,
            reply,
        } => {
            let args = serde_json::json!({ "stepId": step_id, "values": values });
            let result = handle_js_call(rt, ctx, "onSetupSubmit", &args.to_string()).await;
            let _ = reply.send(result);
        }
        SkillMessage::SetupCancel { reply } => {
            let result = handle_js_void_call(rt, ctx, "onSetupCancel", "{}").await;
            let _ = reply.send(result);
        }
        SkillMessage::ListOptions { reply } => {
            let result = handle_js_call(rt, ctx, "onListOptions", "{}").await;
            let _ = reply.send(result);
        }
        SkillMessage::SetOption { name, value, reply } => {
            let args = serde_json::json!({ "name": name, "value": value });
            let result = handle_js_void_call(rt, ctx, "onSetOption", &args.to_string()).await;
            let _ = reply.send(result);
        }
        SkillMessage::SessionStart { session_id, reply } => {
            let args = serde_json::json!({ "sessionId": session_id });
            let result = handle_js_void_call(rt, ctx, "onSessionStart", &args.to_string()).await;
            let _ = reply.send(result);
        }
        SkillMessage::SessionEnd { session_id, reply } => {
            let args = serde_json::json!({ "sessionId": session_id });
            let result = handle_js_void_call(rt, ctx, "onSessionEnd", &args.to_string()).await;
            let _ = reply.send(result);
        }
        SkillMessage::Tick { reply } => {
            let result = handle_js_void_call(rt, ctx, "onTick", "{}").await;
            let _ = reply.send(result);
        }
        SkillMessage::LoadParams { params } => {
            let params_str = serde_json::to_string(&params).unwrap_or_else(|_| "{}".to_string());
            if let Err(e) = handle_js_void_call(rt, ctx, "onLoad", &params_str).await {
                log::warn!(
                    "[skill:{}] onLoad failed (skill may not export it): {}",
                    skill_id,
                    e
                );
            }
        }
        SkillMessage::Error {
            error_type,
            message,
            source,
            recoverable,
        } => {
            let args = serde_json::json!({
                "type": error_type,
                "message": message,
                "source": source,
                "recoverable": recoverable,
            });
            if let Err(e) = handle_js_void_call(rt, ctx, "onError", &args.to_string()).await {
                log::warn!("[skill:{}] onError() handler failed: {e}", skill_id);
            }
        }
        SkillMessage::Rpc {
            method,
            params,
            reply,
        } => {
            // Resolve the optional memory client once for this RPC call.
            // State is registered as MemoryState(Mutex<Option<MemoryClientRef>>), not
            // Option<MemoryClientRef> directly, so we must use the newtype wrapper.
            let memory_client_opt = app_handle.and_then(|ah| {
                ah.try_state::<MemoryState>()
                    .and_then(|s| s.0.lock().ok().and_then(|g| g.clone()))
            });

            let result = match method.as_str() {
                "oauth/complete" => {
                    // Set credential on the oauth bridge + persist to store
                    let cred_json =
                        serde_json::to_string(&params).unwrap_or_else(|_| "null".to_string());
                    let code = format!(
                        r#"(function() {{
                            if (typeof globalThis.oauth !== 'undefined' && globalThis.oauth.__setCredential) {{
                                globalThis.oauth.__setCredential({cred});
                            }}
                            if (typeof globalThis.state !== 'undefined' && globalThis.state.set) {{
                                globalThis.state.set('__oauth_credential', {cred});
                            }}
                        }})()"#,
                        cred = cred_json
                    );
                    ctx.with(|js_ctx| {
                        let _ = js_ctx.eval::<rquickjs::Value, _>(code.as_bytes());
                    })
                    .await;
                    log::info!(
                        "[skill:{}] OAuth credential set and persisted to store",
                        skill_id
                    );
                    let params_str =
                        serde_json::to_string(&params).unwrap_or_else(|_| "{}".to_string());
                    handle_js_call(rt, ctx, "onOAuthComplete", &params_str).await
                }
                "skill/ping" => handle_js_call(rt, ctx, "onPing", "{}").await,
                "skill/sync" => {
                    let result = handle_js_call(rt, ctx, "onSync", "{}").await;
                    // Fire-and-forget: persist published ops state to TinyHumans memory.
                    // Skills publish data via state.set()/setPartial() into ops_state.data,
                    // not as the return value of onSync() (which is typically undefined).
                    let state_snapshot = ops_state.read().data.clone();
                    log::info!(
                        "[memory] store_skill_sync: payload → state_snapshot={:?}",
                        state_snapshot
                    );
                    if !state_snapshot.is_empty() {
                        if let Some(client) = memory_client_opt.clone() {
                            let skill = skill_id.to_string();
                            let content = serde_json::to_string_pretty(&serde_json::Value::Object(
                                state_snapshot,
                            ))
                            .unwrap_or_else(|_| "{}".to_string());
                            let title = format!("{} periodic sync", skill);
                            tokio::spawn(async move {
                                if let Err(e) = client
                                    .store_skill_sync(
                                        &skill, "default", &title, &content, None, None, None,
                                        None, None, None,
                                    )
                                    .await
                                {
                                    log::warn!("[memory] store_skill_sync failed: {e}");
                                }
                            });
                        }
                    }

                    result
                }
                "oauth/revoked" => {
                    // Clear credential: set to empty string so it's clearly "disconnected"
                    let clear_code = r#"(function() {
                        if (typeof globalThis.oauth !== 'undefined' && globalThis.oauth.__setCredential) {
                            globalThis.oauth.__setCredential(null);
                        }
                        if (typeof globalThis.state !== 'undefined' && globalThis.state.set) {
                            globalThis.state.set('__oauth_credential', '');
                        }
                    })()"#;
                    ctx.with(|js_ctx| {
                        let _ = js_ctx.eval::<rquickjs::Value, _>(clear_code.as_bytes());
                    })
                    .await;
                    log::info!("[skill:{}] OAuth credential cleared from store", skill_id);

                    // Fire-and-forget: delete memory for this integration
                    if let Some(client) = memory_client_opt {
                        let skill = skill_id.to_string();
                        let integration_id = params
                            .get("integrationId")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown")
                            .to_string();
                        tokio::spawn(async move {
                            if let Err(e) = client.clear_skill_memory(&skill, &integration_id).await
                            {
                                log::warn!("[memory] clear_skill_memory failed: {e}");
                            } else {
                                log::info!(
                                    "[memory] Cleared memory for {}:{}",
                                    skill,
                                    integration_id
                                );
                            }
                        });
                    }

                    let params_str =
                        serde_json::to_string(&params).unwrap_or_else(|_| "{}".to_string());
                    handle_js_void_call(rt, ctx, "onOAuthRevoked", &params_str)
                        .await
                        .map(|()| serde_json::json!({ "ok": true }))
                }
                _ => {
                    let args = serde_json::json!({ "method": method, "params": params });
                    handle_js_call(rt, ctx, "onRpc", &args.to_string()).await
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
        let message: String = obj.get::<_, String>("message").unwrap_or_default();
        let stack: String = obj.get::<_, String>("stack").unwrap_or_default();

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
/// Handles async (Promise-returning) lifecycle methods (init, start, stop).
async fn call_lifecycle(
    rt: &rquickjs::AsyncRuntime,
    ctx: &rquickjs::AsyncContext,
    name: &str,
) -> Result<(), String> {
    let name = name.to_string();
    let is_promise = ctx.with(|js_ctx| {
        let code = format!(
            r#"(function() {{
                var skill = globalThis.__skill && globalThis.__skill.default
                    ? globalThis.__skill.default
                    : (globalThis.__skill || globalThis);
                var fn = skill.{name} || globalThis.{name};
                if (typeof fn === 'function') {{
                    var result = fn.call(skill);
                    if (result && typeof result.then === 'function') {{
                        globalThis.__pendingLifecycleDone = false;
                        globalThis.__pendingLifecycleError = undefined;
                        result.then(
                            function() {{ globalThis.__pendingLifecycleDone = true; }},
                            function(e) {{
                                globalThis.__pendingLifecycleError = e && e.message ? e.message : String(e);
                                globalThis.__pendingLifecycleDone = true;
                            }}
                        );
                        return "1";
                    }}
                }}
                return "0";
            }})()"#
        );
        match js_ctx.eval::<String, _>(code.as_bytes()) {
            Ok(s) => Ok(s == "1"),
            Err(e) => {
                let detail = format_js_exception(&js_ctx, &e);
                Err(format!("{name}() failed: {detail}"))
            }
        }
    }).await?;

    if is_promise {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);

        loop {
            drive_jobs(rt).await;

            let done = ctx
                .with(|js_ctx| {
                    js_ctx
                        .eval::<bool, _>(b"globalThis.__pendingLifecycleDone === true")
                        .unwrap_or(false)
                })
                .await;

            if done {
                let error = ctx.with(|js_ctx| {
                    let has_error = js_ctx
                        .eval::<bool, _>(b"globalThis.__pendingLifecycleError !== undefined")
                        .unwrap_or(false);
                    let err = if has_error {
                        Some(js_ctx
                            .eval::<String, _>(b"String(globalThis.__pendingLifecycleError)")
                            .unwrap_or_else(|_| "Unknown error".to_string()))
                    } else {
                        None
                    };
                    let _ = js_ctx.eval::<rquickjs::Value, _>(
                        b"delete globalThis.__pendingLifecycleDone; delete globalThis.__pendingLifecycleError;"
                    );
                    err
                }).await;

                if let Some(err_msg) = error {
                    return Err(format!("{name}() rejected: {err_msg}"));
                }
                return Ok(());
            }

            if tokio::time::Instant::now() >= deadline {
                ctx.with(|js_ctx| {
                    let _ = js_ctx.eval::<rquickjs::Value, _>(
                        b"delete globalThis.__pendingLifecycleDone; delete globalThis.__pendingLifecycleError;"
                    );
                }).await;
                return Err(format!("{name}() timed out after 30s"));
            }

            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    Ok(())
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
                    inputSchema: t.inputSchema || t.input_schema || {}
                };
            }));
        })()
    "#;

    // eval with String type hint tells rquickjs to convert the result to a Rust String
    match js_ctx.eval::<String, _>(code) {
        Ok(json_str) => match serde_json::from_str::<Vec<ToolDefinition>>(&json_str) {
            Ok(tools) => {
                state.write().tools = tools;
            }
            Err(e) => {
                log::warn!("[tools] Failed to parse tools JSON: {e}");
            }
        },
        Err(e) => {
            log::warn!("[tools] Failed to extract tools: {e}");
        }
    }
}

/// Start an async tool call.
///
/// Calls the tool's `execute()` and checks if it returns a Promise.
/// - If sync: returns `Ok(Some(ToolResult))` with the immediate result.
/// - If async (Promise): attaches `.then`/`.catch` handlers that store the
///   resolved value in `globalThis.__pendingTool*` globals, and returns
///   `Ok(None)`. The caller should let the event loop drive the QuickJS
///   runtime and poll `__pendingToolDone` for completion.
async fn start_async_tool_call(
    ctx: &rquickjs::AsyncContext,
    tool_name: &str,
    arguments: serde_json::Value,
) -> Result<Option<ToolResult>, String> {
    let args_str =
        serde_json::to_string(&arguments).map_err(|e| format!("Failed to serialize args: {e}"))?;
    let tool_name = tool_name.to_string();

    let eval_result = ctx
        .with(|js_ctx| {
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
                        if (result && typeof result.then === 'function') {{
                            globalThis.__pendingToolResult = undefined;
                            globalThis.__pendingToolError = undefined;
                            globalThis.__pendingToolDone = false;
                            result.then(
                                function(v) {{
                                    globalThis.__pendingToolResult = v;
                                    globalThis.__pendingToolDone = true;
                                }},
                                function(e) {{
                                    globalThis.__pendingToolError = e;
                                    globalThis.__pendingToolDone = true;
                                }}
                            );
                            return "__PROMISE__";
                        }}
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
        })
        .await?;

    if eval_result == "__PROMISE__" {
        // Async — caller should poll __pendingToolDone via the event loop
        Ok(None)
    } else {
        // Sync — return immediately
        Ok(Some(ToolResult {
            content: vec![ToolContent::Text { text: eval_result }],
            is_error: false,
        }))
    }
}

/// Read the result of a completed async tool call from JS globals.
/// Call this only after `globalThis.__pendingToolDone === true`.
async fn read_pending_tool_result(ctx: &rquickjs::AsyncContext) -> Result<ToolResult, String> {
    let result_text = ctx
        .with(|js_ctx| {
            let code = r#"(function() {
                var err = globalThis.__pendingToolError;
                globalThis.__pendingToolError = undefined;
                globalThis.__pendingToolDone = false;
                if (err !== undefined) {
                    var msg = (typeof err === 'object' && err !== null && err.message)
                        ? err.message
                        : String(err);
                    globalThis.__pendingToolResult = undefined;
                    throw new Error(msg);
                }
                var r = globalThis.__pendingToolResult;
                globalThis.__pendingToolResult = undefined;
                if (r === undefined || r === null) return "null";
                if (typeof r === 'object') return JSON.stringify(r);
                return String(r);
            })()"#;

            match js_ctx.eval::<String, _>(code.as_bytes()) {
                Ok(s) => Ok(s),
                Err(e) => {
                    let detail = format_js_exception(&js_ctx, &e);
                    Err(format!("Tool async execution failed: {detail}"))
                }
            }
        })
        .await?;

    Ok(ToolResult {
        content: vec![ToolContent::Text { text: result_text }],
        is_error: false,
    })
}

/// Handle a server event.
/// Handles async (Promise-returning) onServerEvent handlers.
async fn handle_server_event(
    rt: &rquickjs::AsyncRuntime,
    ctx: &rquickjs::AsyncContext,
    event: &str,
    data: serde_json::Value,
) -> Result<(), String> {
    let data_str = serde_json::to_string(&data).unwrap_or_else(|_| "null".to_string());
    let event = event.to_string();

    let is_promise = ctx.with(|js_ctx| {
        let code = format!(
            r#"(function() {{
                var skill = globalThis.__skill && globalThis.__skill.default
                    ? globalThis.__skill.default
                    : (globalThis.__skill || globalThis);
                var fn = skill.onServerEvent || globalThis.onServerEvent;
                if (typeof fn === 'function') {{
                    var result = fn.call(skill, "{}", {});
                    if (result && typeof result.then === 'function') {{
                        globalThis.__pendingEventDone = false;
                        globalThis.__pendingEventError = undefined;
                        result.then(
                            function() {{ globalThis.__pendingEventDone = true; }},
                            function(e) {{
                                globalThis.__pendingEventError = e && e.message ? e.message : String(e);
                                globalThis.__pendingEventDone = true;
                            }}
                        );
                        return "1";
                    }}
                }}
                return "0";
            }})()"#,
            event.replace('"', r#"\""#),
            data_str,
        );

        match js_ctx.eval::<String, _>(code.as_bytes()) {
            Ok(s) => Ok(s == "1"),
            Err(e) => Err(format!("Event handler failed: {e}"))
        }
    }).await?;

    if is_promise {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);

        loop {
            drive_jobs(rt).await;

            let done = ctx
                .with(|js_ctx| {
                    js_ctx
                        .eval::<bool, _>(b"globalThis.__pendingEventDone === true")
                        .unwrap_or(false)
                })
                .await;

            if done {
                let error = ctx.with(|js_ctx| {
                    let has_error = js_ctx
                        .eval::<bool, _>(b"globalThis.__pendingEventError !== undefined")
                        .unwrap_or(false);
                    let err = if has_error {
                        Some(js_ctx
                            .eval::<String, _>(b"String(globalThis.__pendingEventError)")
                            .unwrap_or_else(|_| "Unknown error".to_string()))
                    } else {
                        None
                    };
                    let _ = js_ctx.eval::<rquickjs::Value, _>(
                        b"delete globalThis.__pendingEventDone; delete globalThis.__pendingEventError;"
                    );
                    err
                }).await;

                if let Some(err_msg) = error {
                    return Err(format!("onServerEvent() rejected: {err_msg}"));
                }
                return Ok(());
            }

            if tokio::time::Instant::now() >= deadline {
                ctx.with(|js_ctx| {
                    let _ = js_ctx.eval::<rquickjs::Value, _>(
                        b"delete globalThis.__pendingEventDone; delete globalThis.__pendingEventError;"
                    );
                }).await;
                return Err("onServerEvent() timed out after 30s".to_string());
            }

            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    Ok(())
}

/// Handle a cron trigger.
/// Handles async (Promise-returning) onCronTrigger handlers.
async fn handle_cron_trigger(
    rt: &rquickjs::AsyncRuntime,
    ctx: &rquickjs::AsyncContext,
    schedule_id: &str,
) -> Result<(), String> {
    let schedule_id = schedule_id.to_string();

    let is_promise = ctx.with(|js_ctx| {
        let code = format!(
            r#"(function() {{
                var skill = globalThis.__skill && globalThis.__skill.default
                    ? globalThis.__skill.default
                    : (globalThis.__skill || globalThis);
                var fn = skill.onCronTrigger || globalThis.onCronTrigger;
                if (typeof fn === 'function') {{
                    var result = fn.call(skill, "{}");
                    if (result && typeof result.then === 'function') {{
                        globalThis.__pendingCronDone = false;
                        globalThis.__pendingCronError = undefined;
                        result.then(
                            function() {{ globalThis.__pendingCronDone = true; }},
                            function(e) {{
                                globalThis.__pendingCronError = e && e.message ? e.message : String(e);
                                globalThis.__pendingCronDone = true;
                            }}
                        );
                        return "1";
                    }}
                }}
                return "0";
            }})()"#,
            schedule_id.replace('"', r#"\""#),
        );
        match js_ctx.eval::<String, _>(code.as_bytes()) {
            Ok(s) => Ok(s == "1"),
            Err(e) => Err(format!("Cron trigger failed: {e}"))
        }
    }).await?;

    if is_promise {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);

        loop {
            drive_jobs(rt).await;

            let done = ctx
                .with(|js_ctx| {
                    js_ctx
                        .eval::<bool, _>(b"globalThis.__pendingCronDone === true")
                        .unwrap_or(false)
                })
                .await;

            if done {
                let error = ctx.with(|js_ctx| {
                    let has_error = js_ctx
                        .eval::<bool, _>(b"globalThis.__pendingCronError !== undefined")
                        .unwrap_or(false);
                    let err = if has_error {
                        Some(js_ctx
                            .eval::<String, _>(b"String(globalThis.__pendingCronError)")
                            .unwrap_or_else(|_| "Unknown error".to_string()))
                    } else {
                        None
                    };
                    let _ = js_ctx.eval::<rquickjs::Value, _>(
                        b"delete globalThis.__pendingCronDone; delete globalThis.__pendingCronError;"
                    );
                    err
                }).await;

                if let Some(err_msg) = error {
                    return Err(format!("onCronTrigger() rejected: {err_msg}"));
                }
                return Ok(());
            }

            if tokio::time::Instant::now() >= deadline {
                ctx.with(|js_ctx| {
                    let _ = js_ctx.eval::<rquickjs::Value, _>(
                        b"delete globalThis.__pendingCronDone; delete globalThis.__pendingCronError;"
                    );
                }).await;
                return Err("onCronTrigger() timed out after 30s".to_string());
            }

            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    Ok(())
}

/// Call a JS function on the skill object that returns a JSON value.
/// Handles both sync and async (Promise-returning) functions.
async fn handle_js_call(
    rt: &rquickjs::AsyncRuntime,
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
                    var result = fn.call(skill, {args_json});
                    if (result && typeof result.then === 'function') {{
                        globalThis.__pendingRpcResult = undefined;
                        globalThis.__pendingRpcError = undefined;
                        globalThis.__pendingRpcDone = false;
                        result.then(
                            function(v) {{
                                globalThis.__pendingRpcResult = v;
                                globalThis.__pendingRpcDone = true;
                            }},
                            function(e) {{
                                globalThis.__pendingRpcError = e && e.message ? e.message : String(e);
                                globalThis.__pendingRpcDone = true;
                            }}
                        );
                        return "__PROMISE__";
                    }}
                    return JSON.stringify(result === undefined ? null : result);
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

    if result_text == "__PROMISE__" {
        // Async — drive the QuickJS job queue until the promise resolves
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);

        loop {
            drive_jobs(rt).await;

            let done = ctx
                .with(|js_ctx| {
                    js_ctx
                        .eval::<bool, _>(b"globalThis.__pendingRpcDone === true")
                        .unwrap_or(false)
                })
                .await;

            if done {
                let result = ctx.with(|js_ctx| {
                    let has_error = js_ctx
                        .eval::<bool, _>(b"globalThis.__pendingRpcError !== undefined")
                        .unwrap_or(false);

                    let val = if has_error {
                        let err_msg = js_ctx
                            .eval::<String, _>(b"String(globalThis.__pendingRpcError)")
                            .unwrap_or_else(|_| "Unknown error".to_string());
                        Err(format!("{fn_name}() rejected: {err_msg}"))
                    } else {
                        let json_str = js_ctx
                            .eval::<String, _>(
                                b"JSON.stringify(globalThis.__pendingRpcResult === undefined ? null : globalThis.__pendingRpcResult)"
                            )
                            .unwrap_or_else(|_| "null".to_string());
                        Ok(json_str)
                    };

                    // Clean up globals
                    let _ = js_ctx.eval::<rquickjs::Value, _>(
                        b"delete globalThis.__pendingRpcDone; delete globalThis.__pendingRpcResult; delete globalThis.__pendingRpcError;"
                    );
                    val
                }).await;

                return match result {
                    Ok(json_str) => serde_json::from_str(&json_str)
                        .map_err(|e| format!("{fn_name}() returned invalid JSON: {e}")),
                    Err(e) => Err(e),
                };
            }

            if tokio::time::Instant::now() >= deadline {
                ctx.with(|js_ctx| {
                    let _ = js_ctx.eval::<rquickjs::Value, _>(
                        b"delete globalThis.__pendingRpcDone; delete globalThis.__pendingRpcResult; delete globalThis.__pendingRpcError;"
                    );
                }).await;
                return Err(format!("{fn_name}() timed out after 30s"));
            }

            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    } else {
        serde_json::from_str(&result_text)
            .map_err(|e| format!("{fn_name}() returned invalid JSON: {e}"))
    }
}

/// Call a JS function on the skill object that returns void.
/// Handles both sync and async (Promise-returning) functions.
async fn handle_js_void_call(
    rt: &rquickjs::AsyncRuntime,
    ctx: &rquickjs::AsyncContext,
    fn_name: &str,
    args_json: &str,
) -> Result<(), String> {
    let fn_name = fn_name.to_string();
    let args_json = args_json.to_string();

    let is_promise = ctx.with(|js_ctx| {
        let code = format!(
            r#"(function() {{
                var skill = globalThis.__skill && globalThis.__skill.default
                    ? globalThis.__skill.default
                    : (globalThis.__skill || globalThis);
                var fn = skill.{fn_name} || globalThis.{fn_name};
                if (typeof fn === 'function') {{
                    var result = fn.call(skill, {args_json});
                    if (result && typeof result.then === 'function') {{
                        globalThis.__pendingRpcVoidDone = false;
                        globalThis.__pendingRpcVoidError = undefined;
                        result.then(
                            function() {{ globalThis.__pendingRpcVoidDone = true; }},
                            function(e) {{
                                globalThis.__pendingRpcVoidError = e && e.message ? e.message : String(e);
                                globalThis.__pendingRpcVoidDone = true;
                            }}
                        );
                        return "1";
                    }}
                }}
                return "0";
            }})()"#
        );

        match js_ctx.eval::<String, _>(code.as_bytes()) {
            Ok(s) => Ok(s == "1"),
            Err(e) => {
                let detail = format_js_exception(&js_ctx, &e);
                Err(format!("{fn_name}() failed: {detail}"))
            }
        }
    }).await?;

    if is_promise {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);

        loop {
            drive_jobs(rt).await;

            let done = ctx
                .with(|js_ctx| {
                    js_ctx
                        .eval::<bool, _>(b"globalThis.__pendingRpcVoidDone === true")
                        .unwrap_or(false)
                })
                .await;

            if done {
                let error = ctx.with(|js_ctx| {
                    let has_error = js_ctx
                        .eval::<bool, _>(b"globalThis.__pendingRpcVoidError !== undefined")
                        .unwrap_or(false);
                    let err = if has_error {
                        Some(js_ctx
                            .eval::<String, _>(b"String(globalThis.__pendingRpcVoidError)")
                            .unwrap_or_else(|_| "Unknown error".to_string()))
                    } else {
                        None
                    };
                    let _ = js_ctx.eval::<rquickjs::Value, _>(
                        b"delete globalThis.__pendingRpcVoidDone; delete globalThis.__pendingRpcVoidError;"
                    );
                    err
                }).await;

                if let Some(err_msg) = error {
                    return Err(format!("{fn_name}() rejected: {err_msg}"));
                }
                return Ok(());
            }

            if tokio::time::Instant::now() >= deadline {
                ctx.with(|js_ctx| {
                    let _ = js_ctx.eval::<rquickjs::Value, _>(
                        b"delete globalThis.__pendingRpcVoidDone; delete globalThis.__pendingRpcVoidError;"
                    );
                }).await;
                return Err(format!("{fn_name}() timed out after 30s"));
            }

            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    Ok(())
}

/// Load a persisted OAuth credential from the skill's store and inject it
/// into the JS context so tools have access to the credential.
/// An empty string means "disconnected" — only non-empty values are restored.
async fn restore_oauth_credential(ctx: &rquickjs::AsyncContext, skill_id: &str) {
    let code = r#"(function() {
        if (typeof globalThis.state === 'undefined' || typeof globalThis.oauth === 'undefined') return false;
        var cred = globalThis.state.get('__oauth_credential');
        if (cred && cred !== '' && globalThis.oauth.__setCredential) {
            globalThis.oauth.__setCredential(cred);
            return true;
        }
        return false;
    })()"#;

    let restored = ctx
        .with(|js_ctx| js_ctx.eval::<bool, _>(code.as_bytes()).unwrap_or(false))
        .await;

    if restored {
        log::info!("[skill:{}] Restored OAuth credential from store", skill_id);
    }
}
