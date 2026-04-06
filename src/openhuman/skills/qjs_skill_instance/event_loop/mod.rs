//! Main event loop and message routing for QuickJS skill instances.
//!
//! This module implements the central execution loop that drives a skill's
//! JavaScript environment. It handles timer callbacks, routes incoming system
//! messages (RPCs, tool calls, events), manages asynchronous tool execution,
//! and persists skill state to the OpenHuman memory system.

mod rpc_handlers;
mod webhook_handler;

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
        working_memory::{skills_working_memory_enabled, working_memory_documents_from_sync},
    },
    tool_timeout::{tool_execution_timeout_duration, tool_execution_timeout_secs},
};

use super::js_handlers::{
    call_lifecycle, handle_cron_trigger, handle_js_call, handle_js_void_call, handle_server_event,
    read_pending_tool_result, start_async_tool_call,
};
use super::js_helpers::{
    drive_jobs, restore_auth_credential, restore_client_key, restore_oauth_credential,
};
use super::types::SkillState;

/// Payload queued for the background memory-write worker.
pub(crate) struct MemoryWriteJob {
    /// Reference to the memory client for storage and ingestion.
    client: MemoryClientRef,
    /// ID of the skill that produced the data.
    skill: String,
    /// Title for the persisted document.
    title: String,
    /// Stringified JSON content of the skill's published state.
    content: String,
}

/// Maximum number of memory-write jobs that can be buffered before back-pressure
/// causes `persist_state_to_memory` to drop new writes.
const MEMORY_WRITE_CHANNEL_CAPACITY: usize = 16;

/// Spawn a bounded background worker that consumes `MemoryWriteJob` items.
///
/// This worker performs two main tasks for each job:
/// 1. Stores the raw state snapshot in the memory store using `store_skill_sync`.
/// 2. Ingests the content into the memory graph (vector store) using `ingest_doc`.
///
/// Specialized logic exists for certain skills (like Notion) to ingest content
/// at a more granular level (e.g., per-page).
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

            // Persist the full state blob to the skill's sync history
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

            if skills_working_memory_enabled() {
                let working_memory = working_memory_documents_from_sync(&job.skill, &job.content);
                let mut working_persisted = 0usize;
                let mut working_failed = 0usize;
                for doc in working_memory.documents {
                    let key = doc.key.clone();
                    match job.client.put_doc(doc).await {
                        Ok(_) => {
                            working_persisted += 1;
                        }
                        Err(e) => {
                            working_failed += 1;
                            log::warn!(
                                "[skills-working-memory] put_doc failed for skill='{}' key='{}': {}",
                                job.skill,
                                key,
                                e
                            );
                        }
                    }
                }
                log::info!(
                    "[skills-working-memory] sync_batch skill='{}' title='{}' scalar_fields={} \
                     skipped_sensitive={} prefs={} goals={} constraints={} entities={} \
                     generated_docs={} persisted_docs={} failed_docs={}",
                    job.skill,
                    job.title,
                    working_memory.stats.scalar_fields_seen,
                    working_memory.stats.sensitive_fields_skipped,
                    working_memory.stats.preferences,
                    working_memory.stats.goals,
                    working_memory.stats.constraints,
                    working_memory.stats.entities,
                    working_memory.stats.documents_generated,
                    working_persisted,
                    working_failed,
                );
            } else {
                log::debug!(
                    "[skills-working-memory] disabled by OPENHUMAN_SKILLS_WORKING_MEMORY_ENABLED; skill='{}' title='{}'",
                    job.skill,
                    job.title
                );
            }

            let namespace = format!("skill-{}", job.skill.trim());
            let skill = job.skill.trim().to_lowercase();

            // Perform category-specific ingestion logic
            match skill.as_str() {
                "notion" => {
                    // For Notion, we extract individual pages and ingest them as separate documents
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
                            // Store the individual page as a sub-sync
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
                            // Ingest the page into the vector graph
                            ingest_single_doc(
                                &job.client,
                                &namespace,
                                &page_key,
                                &page.title,
                                &page.content,
                            )
                            .await;
                        }
                    }
                }
                _ => {
                    // Standard skill: ingest the full content as a single document after stripping secrets
                    let safe_content = strip_credentials(&job.content);
                    ingest_single_doc(
                        &job.client,
                        &namespace,
                        &job.title,
                        &job.title,
                        &safe_content,
                    )
                    .await;
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

/// Parse the sync content as JSON and extract individual pages that have `content_text`.
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

/// Remove sensitive fields (like OAuth credentials) from a JSON sync blob before ingestion.
fn strip_credentials(content: &str) -> String {
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(content) else {
        return content.to_string();
    };
    if let Some(obj) = value.as_object_mut() {
        obj.remove("__oauth_credential");
    }
    serde_json::to_string(&value).unwrap_or_else(|_| content.to_string())
}

/// Store and ingest a single document into the memory graph.
async fn ingest_single_doc(
    client: &MemoryClientRef,
    namespace: &str,
    key: &str,
    title: &str,
    content: &str,
) {
    let request = MemoryIngestionRequest {
        document: NamespaceDocumentInput {
            namespace: namespace.to_string(),
            key: key.to_string(),
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

/// Snapshot the skill's published state and queue it for background memory persistence.
pub(crate) fn persist_state_to_memory(
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

/// State for a pending asynchronous tool call being tracked by the event loop.
struct PendingToolCall {
    /// Channel to send the result back to the system.
    reply: tokio::sync::oneshot::Sender<Result<ToolResult, String>>,
    /// Instant when the call is considered timed out.
    deadline: tokio::time::Instant,
}

/// The main event loop that drives the QuickJS runtime.
///
/// This loop runs indefinitely until the skill is stopped or the channel disconnects.
/// It performs the following steps in each iteration:
/// 1. Polls and fires ready timers.
/// 2. Handles incoming messages from the system channel.
/// 3. Drives the QuickJS job queue (promises, microtasks).
/// 4. Checks status of pending asynchronous tool calls.
/// 5. Synchronizes published state between the bridge and the host.
/// 6. Calculates and executes an appropriate sleep duration.
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
    const MAX_IDLE_SLEEP: Duration = Duration::from_millis(100);
    const MIN_SLEEP: Duration = Duration::from_millis(1);
    const TOOL_POLL_SLEEP: Duration = Duration::from_millis(5);

    let memory_write_tx = spawn_memory_write_worker();
    let mut pending_tool: Option<PendingToolCall> = None;

    loop {
        // 1. Poll and fire ready timers (setTimeout / setInterval)
        let (ready_timers, _) = qjs_ops::poll_timers(timer_state);
        for timer_id in ready_timers {
            fire_timer_callback(ctx, timer_id).await;
        }

        // 2. Check for incoming messages (non-blocking try_recv)
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
            Err(mpsc::error::TryRecvError::Empty) => {}
            Err(mpsc::error::TryRecvError::Disconnected) => {
                log::info!(
                    "[skill:{}] Message channel disconnected, stopping",
                    skill_id
                );
                break;
            }
        }

        // 3. Drive QuickJS job queue (process pending promises and microtasks)
        drive_jobs(rt).await;

        // 4. Check if a pending async tool call has completed in the JS environment
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
                // Check for timeout
                let now = tokio::time::Instant::now();
                if now >= ptc.deadline {
                    log::error!(
                        "[skill:{}] Async tool call timed out after {}s",
                        skill_id,
                        tool_execution_timeout_secs()
                    );
                    if let Some(ptc) = pending_tool.take() {
                        let _ = ptc
                            .reply
                            .send(Err("Tool async execution timed out".to_string()));
                    }
                }
            }
        }

        // 5. Sync bridge-level published state to the instance's SkillState
        {
            let mut ops = ops_state.write();
            if ops.dirty {
                ops.dirty = false;
                let new_map: HashMap<String, serde_json::Value> = ops
                    .data
                    .iter()
                    .map(|(k, v): (&String, &serde_json::Value)| (k.clone(), v.clone()))
                    .collect();
                state.write().published_state = new_map;
            }
        }

        // 6. Calculate sleep duration to save CPU cycles when idle
        let sleep_duration = if pending_tool.is_some() {
            // Poll more frequently when waiting for an async tool result
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

        tokio::time::sleep(sleep_duration).await;
    }
}

/// Fire a timer callback in the JavaScript environment.
async fn fire_timer_callback(ctx: &rquickjs::AsyncContext, timer_id: u32) {
    let code = format!("globalThis.__handleTimer({});", timer_id);
    ctx.with(|js_ctx| {
        if let Err(e) = js_ctx.eval::<rquickjs::Value, _>(code.as_bytes()) {
            log::error!("[timer] Callback for timer {} failed: {}", timer_id, e);
        }
    })
    .await;
}

/// Process a single message received from the host system.
///
/// Returns `true` if the message indicates that the event loop should terminate (e.g., Stop).
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

            // Always restore credentials before a tool call to ensure JS has latest tokens
            restore_oauth_credential(ctx, skill_id, data_dir).await;
            restore_auth_credential(ctx, skill_id, data_dir).await;
            restore_client_key(ctx, skill_id, data_dir).await;

            match start_async_tool_call(ctx, &tool_name, arguments).await {
                Ok(Some(sync_result)) => {
                    // Tool returned a result immediately (synchronous)
                    let _ = reply.send(Ok(sync_result));
                }
                Ok(None) => {
                    // Tool returned a Promise, set up tracking for the event loop
                    *pending_tool = Some(PendingToolCall {
                        reply,
                        deadline: tokio::time::Instant::now() + tool_execution_timeout_duration(),
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
            // Trigger the cron handler and persist any updated state to memory
            match handle_cron_trigger(rt, ctx, &schedule_id).await {
                Ok(_) => {
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
                        "[skill:{}] cron trigger '{}' failed: {e}",
                        skill_id,
                        schedule_id,
                    );
                }
            }
        }
        SkillMessage::Stop { reply } => {
            // Clean up lifecycle and clear credentials from the runtime
            let _ = call_lifecycle(rt, ctx, "stop").await;

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
            log::info!("[skill:{}] Stopped", skill_id);
            let _ = reply.send(());

            return true;
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
            let result = webhook_handler::handle_webhook_request(
                rt,
                ctx,
                skill_id,
                correlation_id,
                method,
                path,
                headers,
                query,
                body,
                tunnel_id,
                tunnel_name,
                data_dir,
            )
            .await;
            let _ = reply.send(result);
        }
        SkillMessage::Rpc {
            method,
            params,
            reply,
        } => {
            let result = match method.as_str() {
                "oauth/complete" => {
                    rpc_handlers::handle_oauth_complete(rt, ctx, skill_id, params, data_dir).await
                }
                "skill/ping" => handle_js_call(rt, ctx, "onPing", "{}").await,
                "skill/sync" => {
                    rpc_handlers::handle_sync(
                        rt,
                        ctx,
                        skill_id,
                        ops_state,
                        memory_client,
                        memory_write_tx,
                    )
                    .await
                }
                "oauth/revoked" => {
                    rpc_handlers::handle_oauth_revoked(
                        rt,
                        ctx,
                        skill_id,
                        params,
                        data_dir,
                        memory_client,
                    )
                    .await
                }
                "auth/complete" => {
                    rpc_handlers::handle_auth_complete(rt, ctx, skill_id, params, data_dir).await
                }
                "auth/revoked" => {
                    rpc_handlers::handle_auth_revoked(
                        rt,
                        ctx,
                        skill_id,
                        params,
                        data_dir,
                        memory_client,
                    )
                    .await
                }
                _ => {
                    let args = serde_json::json!({ "method": method, "params": params });
                    handle_js_call(rt, ctx, "onRpc", &args.to_string()).await
                }
            };

            let _ = reply.send(result);
        }
    }
    false
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
        assert_eq!(
            pages.len(),
            1,
            "only page with real content should be extracted"
        );
        assert_eq!(pages[0].id, "page-1");
        assert_eq!(pages[0].title, "Meeting Notes");
        assert_eq!(
            pages[0].content,
            "Attendees: Alice, Bob. Decision: ship by Friday."
        );
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
        assert!(
            pages.is_empty(),
            "empty pages array should produce no results"
        );
    }

    #[test]
    fn extract_pages_skips_non_json() {
        let pages = extract_pages_from_sync("this is plain text, not json");
        assert!(
            pages.is_empty(),
            "non-JSON content should produce no results"
        );
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
        assert!(
            pages.is_empty(),
            "pages without content_text should be skipped"
        );
    }

    #[test]
    fn strip_credentials_removes_oauth() {
        let content = serde_json::json!({
            "__oauth_credential": {"credentialId": "secret123"},
            "auth_status": "authenticated",
            "emails": []
        })
        .to_string();

        let safe = strip_credentials(&content);
        assert!(!safe.contains("secret123"), "credential should be stripped");
        assert!(safe.contains("authenticated"), "other fields preserved");
    }

    #[test]
    fn strip_credentials_passthrough_non_json() {
        let content = "plain text content";
        assert_eq!(strip_credentials(content), content);
    }
}
