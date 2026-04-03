use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::RwLock;
use tokio::sync::mpsc;

use crate::openhuman::{
    memory::{
        MemoryClientRef, MemoryIngestionConfig, MemoryIngestionRequest, NamespaceDocumentInput,
    },
    skills::{
        quickjs_libs::qjs_ops,
        types::{SkillMessage, SkillStatus, ToolResult},
    },
    tool_timeout::{tool_execution_timeout_duration, tool_execution_timeout_secs},
};

use super::js_handlers::{
    call_lifecycle, handle_cron_trigger, handle_js_call, handle_js_void_call, handle_server_event,
    read_pending_tool_result, start_async_tool_call,
};
use super::js_helpers::{drive_jobs, restore_oauth_credential};
use super::types::SkillState;

/// Payload queued for the background memory-write worker.
struct MemoryWriteJob {
    client: MemoryClientRef,
    skill: String,
    title: String,
    content: String,
}

/// Maximum number of memory-write jobs that can be buffered before back-pressure
/// causes `persist_state_to_memory` to drop new writes.
const MEMORY_WRITE_CHANNEL_CAPACITY: usize = 16;

/// Spawn a bounded background worker that consumes `MemoryWriteJob` items,
/// calls `store_skill_sync` to persist the raw document, then runs
/// `ingest_doc` to extract entities and relations into the memory graph.
///
/// When the sync content contains embedded pages (e.g. Notion sync with a
/// `pages` array), each page with `content_text` is stored and ingested as
/// its own document keyed by page ID.  This gives per-page entity extraction,
/// proper dedup on re-sync, and search that returns relevant page chunks
/// rather than the entire sync blob.
///
/// Returns the sender half; dropping it shuts down the worker.
fn spawn_memory_write_worker() -> mpsc::Sender<MemoryWriteJob> {
    let (tx, mut rx) = mpsc::channel::<MemoryWriteJob>(MEMORY_WRITE_CHANNEL_CAPACITY);
    tokio::spawn(async move {
        while let Some(job) = rx.recv().await {
            log::debug!(
                "[memory] store_skill_sync: skill={}, title={}, content_len={}",
                job.skill,
                job.title,
                job.content.len(),
            );
            if let Err(e) = job
                .client
                .store_skill_sync(
                    &job.skill,
                    "default",
                    &job.title,
                    &job.content,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                )
                .await
            {
                log::warn!("[memory] store_skill_sync failed for '{}': {e}", job.title);
                continue;
            }
            log::debug!("[memory] store_skill_sync succeeded for '{}'", job.title);

            let namespace = format!("skill-{}", job.skill.trim());
            let skill = job.skill.trim().to_lowercase();

            // Skill-specific ingestion: each skill type has its own extraction
            // strategy so page/email content is ingested individually rather
            // than as one giant blob.
            match skill.as_str() {
                "notion" => {
                    let pages = extract_pages_from_sync(&job.content);
                    if pages.is_empty() {
                        log::debug!(
                            "[memory] notion: no pages with content in '{}', skipping ingestion",
                            job.title,
                        );
                    } else {
                        log::debug!(
                            "[memory] notion: ingesting {} pages individually",
                            pages.len(),
                        );
                        for page in &pages {
                            let page_key = format!("page-{}", page.id);
                            if let Err(e) = job
                                .client
                                .store_skill_sync(
                                    &job.skill,
                                    "default",
                                    &page.title,
                                    &page.content,
                                    Some("notion".to_string()),
                                    Some(serde_json::json!({
                                        "page_id": page.id,
                                        "url": page.url,
                                    })),
                                    None,
                                    None,
                                    None,
                                    Some(page_key.clone()),
                                )
                                .await
                            {
                                log::warn!(
                                    "[memory] notion: store page '{}' failed: {e}",
                                    page.title,
                                );
                                continue;
                            }
                            ingest_single_doc(&job.client, &namespace, &page.title, &page.content)
                                .await;
                        }
                    }
                }
                // Other skills: ingest the full content as a single document.
                _ => {
                    ingest_single_doc(&job.client, &namespace, &job.title, &job.content).await;
                }
            }
        }
        log::debug!("[memory] memory-write worker shutting down");
    });
    tx
}

/// A page extracted from a skill sync blob.
struct SyncPage {
    id: String,
    title: String,
    url: String,
    content: String,
}

/// Parse the sync content as JSON and extract individual pages that have
/// `content_text`.  Returns an empty vec if the content isn't JSON or has
/// no pages with content.
fn extract_pages_from_sync(content: &str) -> Vec<SyncPage> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(content) else {
        return Vec::new();
    };
    let Some(pages) = value.get("pages").and_then(|p| p.as_array()) else {
        return Vec::new();
    };
    pages
        .iter()
        .filter_map(|page| {
            let content_text = page.get("content_text")?.as_str()?;
            if content_text.trim().is_empty() {
                return None;
            }
            Some(SyncPage {
                id: page.get("id")?.as_str()?.to_string(),
                title: page
                    .get("title")
                    .and_then(|t| t.as_str())
                    .unwrap_or("Untitled")
                    .to_string(),
                url: page
                    .get("url")
                    .and_then(|u| u.as_str())
                    .unwrap_or("")
                    .to_string(),
                content: content_text.to_string(),
            })
        })
        .collect()
}

/// Store and ingest a single document into the memory graph.
async fn ingest_single_doc(
    client: &MemoryClientRef,
    namespace: &str,
    title: &str,
    content: &str,
) {
    let request = MemoryIngestionRequest {
        document: NamespaceDocumentInput {
            namespace: namespace.to_string(),
            key: title.to_string(),
            title: title.to_string(),
            content: content.to_string(),
            source_type: "doc".to_string(),
            priority: "medium".to_string(),
            tags: Vec::new(),
            metadata: serde_json::json!({}),
            category: "core".to_string(),
            session_id: None,
            document_id: None,
        },
        config: MemoryIngestionConfig::default(),
    };
    log::debug!("[memory] ingest_doc starting for '{}'", title);
    match client.ingest_doc(request).await {
        Ok(result) => {
            log::info!(
                "[memory] ingest_doc succeeded for '{}': {} entities, {} relations, {} chunks",
                title,
                result.entity_count,
                result.relation_count,
                result.chunk_count,
            );
        }
        Err(e) => {
            log::warn!(
                "[memory] ingest_doc failed for '{}' (non-fatal): {e}",
                title,
            );
        }
    }
}

/// Snapshot the skill's published ops state and queue it for memory persistence.
///
/// Called after sync, cron, and tick handlers so that data published via
/// `state.set()` / `state.setPartial()` during the JS handler is written to the
/// local memory store (SQLite + vector embeddings).
///
/// Writes are dispatched to a bounded background worker (see
/// [`spawn_memory_write_worker`]).  If the worker is busy the write is dropped
/// rather than blocking the event loop.
fn persist_state_to_memory(
    skill_id: &str,
    title_suffix: &str,
    ops_state: &Arc<RwLock<qjs_ops::SkillState>>,
    memory_client: &Option<MemoryClientRef>,
    memory_write_tx: &mpsc::Sender<MemoryWriteJob>,
) {
    let state_snapshot = ops_state.read().data.clone();
    log::debug!(
        "[skill:{}] persist_state_to_memory({}): {} keys in snapshot",
        skill_id,
        title_suffix,
        state_snapshot.len(),
    );
    if state_snapshot.is_empty() {
        return;
    }
    let Some(client) = memory_client.clone() else {
        log::debug!(
            "[skill:{}] persist_state_to_memory: no memory client available, skipping",
            skill_id,
        );
        return;
    };
    let skill = skill_id.to_string();
    let content = serde_json::to_string_pretty(&serde_json::Value::Object(state_snapshot))
        .unwrap_or_else(|_| "{}".to_string());
    let title = format!("{} {}", skill, title_suffix);
    if let Err(e) = memory_write_tx.try_send(MemoryWriteJob {
        client,
        skill,
        title: title.clone(),
        content,
    }) {
        log::warn!(
            "[memory] persist_state_to_memory: channel full, dropping write for '{title}': {e}"
        );
    }
}

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
    data_dir: &std::path::Path,
) {
    // Maximum sleep duration when no timers are pending
    const MAX_IDLE_SLEEP: Duration = Duration::from_millis(100);
    // Minimum sleep to prevent busy-spinning
    const MIN_SLEEP: Duration = Duration::from_millis(1);
    // Faster polling when waiting for an async tool call
    const TOOL_POLL_SLEEP: Duration = Duration::from_millis(5);

    // Bounded background worker for memory writes — limits concurrent in-flight
    // store_skill_sync calls and applies backpressure when the channel is full.
    let memory_write_tx = spawn_memory_write_worker();

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
                    data_dir,
                    &memory_write_tx,
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
                log::info!("[skill:{}] Pending async tool call completed", skill_id);
                let result = read_pending_tool_result(ctx).await;
                if let Some(ptc) = pending_tool.take() {
                    log::info!(
                        "[skill:{}] Sending tool result (is_err={})",
                        skill_id,
                        result.is_err()
                    );
                    let _ = ptc.reply.send(result);
                }
            } else if let Some(ref ptc) = pending_tool {
                let remaining = ptc
                    .deadline
                    .saturating_duration_since(tokio::time::Instant::now());
                if remaining.as_secs() % 10 == 0 && remaining.as_millis() % 10000 < 100 {
                    log::debug!(
                        "[skill:{}] Still waiting for async tool result ({:.0}s remaining)",
                        skill_id,
                        remaining.as_secs_f32()
                    );
                }
                if tokio::time::Instant::now() >= ptc.deadline {
                    log::error!(
                        "[skill:{}] Async tool call timed out after {}s",
                        skill_id,
                        tool_execution_timeout_secs()
                    );
                    // Dump JS error state for debugging
                    let error_info = ctx
                        .with(|js_ctx| {
                            js_ctx
                                .eval::<String, _>(
                                    b"JSON.stringify({ done: globalThis.__pendingToolDone, result: typeof globalThis.__pendingToolResult, error: globalThis.__pendingToolError ? String(globalThis.__pendingToolError) : null })",
                                )
                                .unwrap_or_else(|_| "eval failed".to_string())
                        })
                        .await;
                    log::error!(
                        "[skill:{}] Tool timeout debug state: {}",
                        skill_id,
                        error_info
                    );
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
    data_dir: &std::path::Path,
    memory_write_tx: &mpsc::Sender<MemoryWriteJob>,
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
            restore_oauth_credential(ctx, skill_id, data_dir).await;
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
                        deadline: tokio::time::Instant::now() + tool_execution_timeout_duration(),
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
            match handle_cron_trigger(rt, ctx, &schedule_id).await {
                Ok(_) => {
                    // Persist state to memory after successful cron-triggered sync
                    persist_state_to_memory(
                        skill_id,
                        &format!("cron sync ({})", schedule_id),
                        ops_state,
                        memory_client,
                        memory_write_tx,
                    );
                }
                Err(e) => {
                    log::warn!(
                        "[skill:{}] cron trigger '{}' failed, skipping memory persistence: {e}",
                        skill_id,
                        schedule_id,
                    );
                }
            }
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
            if result.is_ok() {
                // Persist any state published during tick to memory
                persist_state_to_memory(
                    skill_id,
                    "tick sync",
                    ops_state,
                    memory_client,
                    memory_write_tx,
                );
            }
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
        SkillMessage::WebhookRequest {
            correlation_id,
            method,
            path,
            headers,
            query,
            body,
            tunnel_id,
            tunnel_name,
            reply,
        } => {
            log::info!(
                "[skill:{}] event_loop: WebhookRequest {} {} (tunnel={})",
                skill_id,
                method,
                path,
                tunnel_id,
            );

            // Restore OAuth credential in case the handler needs authenticated API calls
            restore_oauth_credential(ctx, skill_id, data_dir).await;

            let args = serde_json::json!({
                "correlationId": correlation_id,
                "method": method,
                "path": path,
                "headers": headers,
                "query": query,
                "body": body,
                "tunnelId": tunnel_id,
                "tunnelName": tunnel_name,
            });

            match handle_js_call(rt, ctx, "onWebhookRequest", &args.to_string()).await {
                Ok(response_val) => {
                    use crate::openhuman::webhooks::WebhookResponseData;

                    let status_code = response_val
                        .get("statusCode")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(200) as u16;
                    let resp_headers: HashMap<String, String> = response_val
                        .get("headers")
                        .map(|v| match serde_json::from_value(v.clone()) {
                            Ok(h) => h,
                            Err(e) => {
                                log::warn!(
                                    "[skill] Failed to parse webhook response headers: {e}, raw: {v}"
                                );
                                HashMap::new()
                            }
                        })
                        .unwrap_or_default();
                    let resp_body = response_val
                        .get("body")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();

                    log::debug!(
                        "[skill:{}] event_loop: WebhookRequest handled → status {}",
                        skill_id,
                        status_code,
                    );

                    let _ = reply.send(Ok(WebhookResponseData {
                        correlation_id,
                        status_code,
                        headers: resp_headers,
                        body: resp_body,
                    }));
                }
                Err(e) => {
                    log::warn!(
                        "[skill:{}] event_loop: onWebhookRequest failed: {}",
                        skill_id,
                        e,
                    );
                    let _ = reply.send(Err(e));
                }
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
                    // Set credential on the oauth bridge + in-memory state
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

                    // Persist credential to disk so it survives restarts
                    let cred_path = data_dir.join("oauth_credential.json");
                    if let Err(e) = std::fs::write(&cred_path, &cred_json) {
                        log::error!(
                            "[skill:{}] Failed to persist OAuth credential: {e}",
                            skill_id
                        );
                    } else {
                        log::info!(
                            "[skill:{}] OAuth credential persisted to {}",
                            skill_id,
                            cred_path.display()
                        );
                    }
                    let params_str =
                        serde_json::to_string(&params).unwrap_or_else(|_| "{}".to_string());
                    handle_js_call(rt, ctx, "onOAuthComplete", &params_str).await
                }
                "skill/ping" => handle_js_call(rt, ctx, "onPing", "{}").await,
                "skill/sync" => {
                    let result = handle_js_call(rt, ctx, "onSync", "{}").await;
                    if result.is_ok() {
                        // Persist published ops state to memory after onSync() succeeds.
                        // Skills publish data via state.set()/setPartial() into ops_state.data,
                        // not as the return value of onSync() (which is typically undefined).
                        persist_state_to_memory(
                            skill_id,
                            "periodic sync",
                            ops_state,
                            &memory_client_opt,
                            memory_write_tx,
                        );
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

                    // Remove persisted credential file
                    let cred_path = data_dir.join("oauth_credential.json");
                    let _ = std::fs::remove_file(&cred_path);
                    log::info!(
                        "[skill:{}] OAuth credential cleared from store and disk",
                        skill_id
                    );

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_pages_from_notion_sync() {
        let content = serde_json::json!({
            "snapshot_version": "notion-sync-v2",
            "pages": [
                {
                    "id": "page-1",
                    "title": "Meeting Notes",
                    "url": "https://notion.so/page-1",
                    "content_text": "Attendees: Alice, Bob. Decision: ship by Friday.",
                    "has_content": true
                },
                {
                    "id": "page-2",
                    "title": "Empty Page",
                    "url": "https://notion.so/page-2",
                    "content_text": "",
                    "has_content": false
                },
                {
                    "id": "page-3",
                    "title": "Whitespace Only",
                    "url": "https://notion.so/page-3",
                    "content_text": "   ",
                    "has_content": false
                }
            ]
        })
        .to_string();

        let pages = extract_pages_from_sync(&content);
        assert_eq!(pages.len(), 1, "only page with real content should be extracted");
        assert_eq!(pages[0].id, "page-1");
        assert_eq!(pages[0].title, "Meeting Notes");
        assert_eq!(pages[0].content, "Attendees: Alice, Bob. Decision: ship by Friday.");
    }

    #[test]
    fn extract_pages_skips_gmail_sync() {
        let content = serde_json::json!({
            "__oauth_credential": {"credentialId": "abc123"},
            "auth_status": "authenticated",
            "emails": [],
            "totalEmails": 0,
            "syncInProgress": false
        })
        .to_string();

        let pages = extract_pages_from_sync(&content);
        assert!(pages.is_empty(), "gmail sync data should not produce pages");
    }

    #[test]
    fn extract_pages_skips_ops_state_with_empty_pages() {
        let content = serde_json::json!({
            "auth_status": "authenticated",
            "pages": [],
            "totalPages": 0,
            "syncInProgress": false
        })
        .to_string();

        let pages = extract_pages_from_sync(&content);
        assert!(pages.is_empty(), "empty pages array should produce no results");
    }

    #[test]
    fn extract_pages_skips_non_json() {
        let pages = extract_pages_from_sync("this is plain text, not json");
        assert!(pages.is_empty(), "non-JSON content should produce no results");
    }

    #[test]
    fn extract_pages_skips_pages_without_content_text() {
        let content = serde_json::json!({
            "pages": [
                {"id": "page-1", "title": "No Content Field"},
                {"id": "page-2", "title": "Null Content", "content_text": null}
            ]
        })
        .to_string();

        let pages = extract_pages_from_sync(&content);
        assert!(pages.is_empty(), "pages without content_text should be skipped");
    }
}
