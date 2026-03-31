use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::RwLock;
use tokio::sync::mpsc;

use crate::openhuman::memory::MemoryClientRef;
use crate::openhuman::skills::quickjs_libs::qjs_ops;
use crate::openhuman::skills::types::{SkillMessage, SkillStatus, ToolResult};

use super::js_handlers::{
    call_lifecycle, handle_cron_trigger, handle_js_call, handle_js_void_call, handle_server_event,
    read_pending_tool_result, start_async_tool_call,
};
use super::js_helpers::{drive_jobs, restore_oauth_credential};
use super::types::SkillState;

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
/// 5. Syncs published state from ops → instance
/// 6. Sleeps efficiently when idle
pub(crate) async fn run_event_loop(
    rt: &rquickjs::AsyncRuntime,
    ctx: &rquickjs::AsyncContext,
    rx: &mut mpsc::Receiver<SkillMessage>,
    state: &Arc<RwLock<SkillState>>,
    skill_id: &str,
    timer_state: &Arc<RwLock<qjs_ops::TimerState>>,
    ops_state: &Arc<RwLock<qjs_ops::SkillState>>,
    memory_client: Option<MemoryClientRef>,
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
                    &memory_client,
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
                state.write().published_state = new_map;
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
    memory_client: &Option<MemoryClientRef>,
    ops_state: &Arc<RwLock<qjs_ops::SkillState>>,
) -> bool {
    match msg {
        SkillMessage::CallTool {
            tool_name,
            arguments,
            reply,
        } => {
            log::info!(
                "[skill:{}] event_loop: CallTool '{}' received",
                skill_id,
                tool_name
            );

            // Lazy-load persisted OAuth credential before calling the tool
            restore_oauth_credential(ctx, skill_id).await;
            log::debug!(
                "[skill:{}] event_loop: OAuth credential restored for tool '{}'",
                skill_id,
                tool_name
            );

            // Start the async tool execution. The JS code stores the result
            // in globals when done. The main event loop checks for completion.
            match start_async_tool_call(ctx, &tool_name, arguments).await {
                Ok(Some(sync_result)) => {
                    log::info!(
                        "[skill:{}] event_loop: tool '{}' completed synchronously (blocks={})",
                        skill_id,
                        tool_name,
                        sync_result.content.len()
                    );
                    let _ = reply.send(Ok(sync_result));
                }
                Ok(None) => {
                    log::info!(
                        "[skill:{}] event_loop: tool '{}' returned Promise, waiting async",
                        skill_id,
                        tool_name
                    );
                    *pending_tool = Some(PendingToolCall {
                        reply,
                        deadline: tokio::time::Instant::now() + Duration::from_secs(120),
                    });
                }
                Err(e) => {
                    log::error!(
                        "[skill:{}] event_loop: tool '{}' failed: {}",
                        skill_id,
                        tool_name,
                        e
                    );
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
            let memory_client_opt = memory_client.clone();

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
