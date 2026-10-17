//! In-process `WebChannelEvent` broadcast bus, plus the `DomainEvent`
//! surface subscribers (approval/plan-review, artifact, egress) that bridge
//! domain events onto it for Socket.IO and the JSON-RPC `/events` stream.

use async_trait::async_trait;
use std::sync::{Arc, OnceLock};

use crate::core::events::DomainEvent;
use crate::web_chat::WebChannelEvent;
use tinybus::EventHandler;
use tinybus::SubscriptionHandle;

pub use super::event_bus_channel::{
    publish_web_channel_event, publish_web_channel_event_in, subscribe_web_channel_events,
};

static APPROVAL_SURFACE_HANDLE: OnceLock<SubscriptionHandle> = OnceLock::new();

pub fn register_approval_surface_subscriber() {
    if APPROVAL_SURFACE_HANDLE.get().is_some() {
        return;
    }
    match crate::core::bus::BUS.subscribe(Arc::new(ApprovalSurfaceSubscriber)) {
        Some(handle) => {
            let _ = APPROVAL_SURFACE_HANDLE.set(handle);
            log::info!(
                "[web-channel] approval-surface subscriber registered (domains=approval,plan_review) — bridges ApprovalRequested → approval_request and PlanReviewRequested → plan_review_request socket events"
            );
        }
        None => {
            log::warn!(
                "[web-channel] failed to register approval-surface subscriber — bus not initialized"
            );
        }
    }
}

static ARTIFACT_SURFACE_HANDLE: OnceLock<SubscriptionHandle> = OnceLock::new();

pub fn register_artifact_surface_subscriber() {
    if ARTIFACT_SURFACE_HANDLE.get().is_some() {
        return;
    }
    match crate::core::bus::BUS.subscribe(Arc::new(ArtifactSurfaceSubscriber)) {
        Some(handle) => {
            let _ = ARTIFACT_SURFACE_HANDLE.set(handle);
            log::info!(
                "[web-channel] artifact-surface subscriber registered (domain=artifact) — will bridge ArtifactPending/Ready/Failed → artifact_pending/artifact_ready/artifact_failed socket events"
            );
        }
        None => {
            log::warn!(
                "[web-channel] failed to register artifact-surface subscriber — bus not initialized"
            );
        }
    }
}

static MEMORY_ACTIVITY_SURFACE_HANDLE: OnceLock<SubscriptionHandle> = OnceLock::new();

/// Registers the memory-activity surface bridge
/// (`DomainEvent::MemoryStored`/`MemoryRecalled` → `memory_activity`
/// web-channel events). Idempotent (OnceLock-guarded).
pub fn register_memory_activity_surface_subscriber() {
    if MEMORY_ACTIVITY_SURFACE_HANDLE.get().is_some() {
        return;
    }
    match crate::core::bus::BUS.subscribe(Arc::new(MemoryActivitySurfaceSubscriber)) {
        Some(handle) => {
            let _ = MEMORY_ACTIVITY_SURFACE_HANDLE.set(handle);
            log::info!(
                "[web-channel] memory-activity-surface subscriber registered (domain=memory) — will bridge MemoryStored/MemoryRecalled → memory_activity socket events"
            );
        }
        None => {
            log::warn!(
                "[web-channel] failed to register memory-activity-surface subscriber — bus not initialized"
            );
        }
    }
}

/// Longest clipped preview of a recall query carried on a `memory_activity`
/// event. Deliberately short and deliberately not the whole query — see
/// module docs on why memory content/queries never reach the web channel
/// verbatim.
const MEMORY_ACTIVITY_QUERY_PREVIEW_CHARS: usize = 40;

/// Bridges `DomainEvent::MemoryStored`/`MemoryRecalled` — published once per
/// `memory_store`/`memory_recall` **tool call** (`memory::tools::store`/
/// `recall`), not per driver read — onto a `memory_activity` web-channel
/// event so the chat surface can show a brief "remembered"/"recalled N"
/// indicator.
///
/// Routing: the events carry the `thread_id`/`client_id` of the turn whose tool
/// call raised them (taken from that turn's explicit approval context), and the
/// subscriber drops an event that has none (CLI / cron / sub-agent paths — no
/// client to fan out to). Only ever carries
/// `key`/`category`/`namespace` (never stored content) and a short, clipped
/// preview of the recall query (never the full query text).
struct MemoryActivitySurfaceSubscriber;

#[async_trait]
impl EventHandler<DomainEvent> for MemoryActivitySurfaceSubscriber {
    fn name(&self) -> &str {
        "web_chat::memory_activity_surface"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["memory"])
    }

    async fn handle(&self, event: &DomainEvent) {
        let (event_name, thread_id, client_id, args) = match event {
            DomainEvent::MemoryStored {
                key,
                category,
                namespace,
                thread_id,
                client_id,
            } => (
                "stored",
                thread_id,
                client_id,
                serde_json::json!({
                    "kind": "stored",
                    "key": key,
                    "category": category,
                    "namespace": namespace,
                }),
            ),
            DomainEvent::MemoryRecalled {
                query,
                hit_count,
                thread_id,
                client_id,
            } => {
                let preview: String = query
                    .chars()
                    .take(MEMORY_ACTIVITY_QUERY_PREVIEW_CHARS)
                    .collect();
                let truncated = query.chars().count() > MEMORY_ACTIVITY_QUERY_PREVIEW_CHARS;
                (
                    "recalled",
                    thread_id,
                    client_id,
                    serde_json::json!({
                        "kind": "recalled",
                        "query_preview": if truncated { format!("{preview}…") } else { preview },
                        "hit_count": hit_count,
                    }),
                )
            }
            _ => return,
        };
        let (Some(thread_id), Some(client_id)) = (thread_id.clone(), client_id.clone()) else {
            log::debug!("[web-channel] memory-activity-surface skip {event_name}: no chat context");
            return;
        };
        log::debug!(
            "[web-channel] memory-activity-surface emitting memory_activity kind={event_name} thread_id={thread_id} client_id={client_id}"
        );
        publish_web_channel_event(WebChannelEvent {
            event: "memory_activity".to_string(),
            client_id,
            thread_id,
            args: Some(args),
            ..Default::default()
        });
    }
}

static AGENT_SURFACE_HANDLE: OnceLock<SubscriptionHandle> = OnceLock::new();

/// Register the agent-surface bridge that turns thread-goal, thread-todo, and
/// run-queue lifecycle events (domain `"agent"`) into `thread_goal_updated` /
/// `thread_goal_cleared` / `thread_todos_changed` / `queue_item_queued` /
/// `queue_item_delivered` web-channel events (C3: goals/todos/queue UI).
/// Idempotent via a process-level [`OnceLock`].
pub fn register_agent_surface_subscriber() {
    if AGENT_SURFACE_HANDLE.get().is_some() {
        return;
    }
    match crate::core::bus::BUS.subscribe(Arc::new(AgentSurfaceSubscriber)) {
        Some(handle) => {
            let _ = AGENT_SURFACE_HANDLE.set(handle);
            log::info!(
                "[web-channel] agent-surface subscriber registered (domain=agent) — bridges ThreadGoalUpdated/ThreadGoalCleared/ThreadTodosChanged/RunQueue* → thread_goal_updated/thread_goal_cleared/thread_todos_changed/queue_item_queued/queue_item_delivered socket events"
            );
        }
        None => {
            log::warn!(
                "[web-channel] failed to register agent-surface subscriber — bus not initialized"
            );
        }
    }
}

/// Bridge thread-goal / thread-todo / run-queue [`DomainEvent`]s onto the web
/// channel. These events carry only a `thread_id` (no `client_id` — a goal,
/// todo list, or queue is thread-scoped, not client-scoped), so every emitted
/// [`WebChannelEvent`] uses an empty `client_id`; `emit_web_channel_event`
/// still routes it to the `thread:<id>` room because room selection only
/// requires a non-empty `thread_id` and a `client_id` that isn't `"system"`.
struct AgentSurfaceSubscriber;

#[async_trait]
impl EventHandler<DomainEvent> for AgentSurfaceSubscriber {
    fn name(&self) -> &str {
        "web_chat::agent_surface"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["agent"])
    }

    async fn handle(&self, event: &DomainEvent) {
        match event {
            DomainEvent::ThreadGoalUpdated {
                thread_id, goal, ..
            } => {
                log::debug!(
                    "[web-channel] agent-surface emitting thread_goal_updated thread_id={thread_id}"
                );
                publish_web_channel_event(WebChannelEvent {
                    event: "thread_goal_updated".to_string(),
                    client_id: String::new(),
                    thread_id: thread_id.clone(),
                    goal: goal.clone(),
                    ..Default::default()
                });
            }
            DomainEvent::ThreadGoalCleared { thread_id } => {
                log::debug!(
                    "[web-channel] agent-surface emitting thread_goal_cleared thread_id={thread_id}"
                );
                publish_web_channel_event(WebChannelEvent {
                    event: "thread_goal_cleared".to_string(),
                    client_id: String::new(),
                    thread_id: thread_id.clone(),
                    ..Default::default()
                });
            }
            DomainEvent::ThreadTodosChanged { thread_id, todos } => {
                log::debug!(
                    "[web-channel] agent-surface emitting thread_todos_changed thread_id={thread_id}"
                );
                publish_web_channel_event(WebChannelEvent {
                    event: "thread_todos_changed".to_string(),
                    client_id: String::new(),
                    thread_id: thread_id.clone(),
                    todos: Some(todos.clone()),
                    ..Default::default()
                });
            }
            DomainEvent::ThreadRunModeChanged { thread_id, mode } => {
                log::debug!(
                    "[web-channel] agent-surface emitting run_mode_changed thread_id={thread_id} mode={mode}"
                );
                publish_web_channel_event(WebChannelEvent {
                    event: "run_mode_changed".to_string(),
                    client_id: String::new(),
                    thread_id: thread_id.clone(),
                    message: Some(mode.clone()),
                    ..Default::default()
                });
            }
            DomainEvent::RunQueueMessageQueued {
                thread_id,
                item_id,
                text_preview,
                ..
            }
            | DomainEvent::RunQueueSteerRequeued {
                thread_id,
                item_id,
                text_preview,
                ..
            } => {
                let Some(item_id) = item_id.clone() else {
                    return;
                };
                log::debug!(
                    "[web-channel] agent-surface emitting queue_item_queued thread_id={thread_id} item_id={item_id}"
                );
                publish_web_channel_event(WebChannelEvent {
                    event: "queue_item_queued".to_string(),
                    client_id: String::new(),
                    thread_id: thread_id.clone(),
                    queue_item: Some(crate::web_chat::QueueItemPayload {
                        id: item_id,
                        lane: None,
                        text_preview: text_preview.clone(),
                    }),
                    ..Default::default()
                });
            }
            DomainEvent::RunQueueMessageDelivered {
                thread_id,
                mode,
                item_id,
                text_preview,
                ..
            } => {
                let Some(item_id) = item_id.clone() else {
                    return;
                };
                let lane = Some(mode.clone());
                log::debug!(
                    "[web-channel] agent-surface emitting queue_item_delivered thread_id={thread_id} item_id={item_id}"
                );
                publish_web_channel_event(WebChannelEvent {
                    event: "queue_item_delivered".to_string(),
                    client_id: String::new(),
                    thread_id: thread_id.clone(),
                    queue_item: Some(crate::web_chat::QueueItemPayload {
                        id: item_id,
                        lane,
                        text_preview: text_preview.clone(),
                    }),
                    ..Default::default()
                });
            }
            DomainEvent::RunQueueFollowupDispatched {
                thread_id,
                item_id,
                text_preview,
                ..
            }
            | DomainEvent::RunQueueInterrupted {
                thread_id,
                item_id,
                text_preview,
                ..
            } => {
                let Some(item_id) = item_id.clone() else {
                    return;
                };
                let lane = None;
                log::debug!(
                    "[web-channel] agent-surface emitting queue_item_delivered thread_id={thread_id} item_id={item_id}"
                );
                publish_web_channel_event(WebChannelEvent {
                    event: "queue_item_delivered".to_string(),
                    client_id: String::new(),
                    thread_id: thread_id.clone(),
                    queue_item: Some(crate::web_chat::QueueItemPayload {
                        id: item_id,
                        lane,
                        text_preview: text_preview.clone(),
                    }),
                    ..Default::default()
                });
            }
            _ => {}
        }
    }
}

struct ArtifactSurfaceSubscriber;

#[async_trait]
impl EventHandler<DomainEvent> for ArtifactSurfaceSubscriber {
    fn name(&self) -> &str {
        "web_chat::artifact_surface"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["artifact"])
    }

    async fn handle(&self, event: &DomainEvent) {
        match event {
            DomainEvent::ArtifactReady {
                artifact_id,
                kind,
                title,
                workspace_dir,
                path,
                size_bytes,
                thread_id,
                client_id,
                tool_call_id,
                request_id,
            } => {
                let (Some(thread_id), Some(client_id)) = (thread_id, client_id) else {
                    log::debug!(
                        "[web-channel] artifact-surface skip ArtifactReady id={artifact_id}: no chat context"
                    );
                    return;
                };
                log::info!(
                    "[web-channel] artifact-surface emitting artifact_ready id={artifact_id} kind={kind} thread_id={thread_id} client_id={client_id} tool_call_id={tool_call_id:?}"
                );
                publish_web_channel_event(WebChannelEvent {
                    event: "artifact_ready".to_string(),
                    client_id: client_id.clone(),
                    thread_id: thread_id.clone(),
                    tool_call_id: tool_call_id.clone(),
                    turn_request_id: request_id.clone(),
                    args: Some(serde_json::json!({
                        "artifact_id": artifact_id,
                        "kind": kind,
                        "title": title,
                        "workspace_dir": workspace_dir,
                        "path": path,
                        "size_bytes": size_bytes,
                    })),
                    ..Default::default()
                });
            }
            DomainEvent::ArtifactFailed {
                artifact_id,
                kind,
                title,
                workspace_dir,
                error,
                thread_id,
                client_id,
                tool_call_id,
                request_id,
            } => {
                let (Some(thread_id), Some(client_id)) = (thread_id, client_id) else {
                    log::debug!(
                        "[web-channel] artifact-surface skip ArtifactFailed id={artifact_id}: no chat context"
                    );
                    return;
                };
                log::warn!(
                    "[web-channel] artifact-surface emitting artifact_failed id={artifact_id} kind={kind} thread_id={thread_id} client_id={client_id} tool_call_id={tool_call_id:?} error_len={}",
                    error.len()
                );
                publish_web_channel_event(WebChannelEvent {
                    event: "artifact_failed".to_string(),
                    client_id: client_id.clone(),
                    thread_id: thread_id.clone(),
                    tool_call_id: tool_call_id.clone(),
                    turn_request_id: request_id.clone(),
                    args: Some(serde_json::json!({
                        "artifact_id": artifact_id,
                        "kind": kind,
                        "title": title,
                        "workspace_dir": workspace_dir,
                        "error": error,
                    })),
                    ..Default::default()
                });
            }
            DomainEvent::ArtifactPending {
                artifact_id,
                kind,
                title,
                workspace_dir,
                path,
                thread_id,
                client_id,
                tool_call_id,
                request_id,
            } => {
                let (Some(thread_id), Some(client_id)) = (thread_id, client_id) else {
                    log::debug!(
                        "[web-channel] artifact-surface skip ArtifactPending id={artifact_id}: no chat context"
                    );
                    return;
                };
                log::info!(
                    "[web-channel] artifact-surface emitting artifact_pending id={artifact_id} kind={kind} thread_id={thread_id} client_id={client_id} tool_call_id={tool_call_id:?}"
                );
                publish_web_channel_event(WebChannelEvent {
                    event: "artifact_pending".to_string(),
                    client_id: client_id.clone(),
                    thread_id: thread_id.clone(),
                    tool_call_id: tool_call_id.clone(),
                    turn_request_id: request_id.clone(),
                    args: Some(serde_json::json!({
                        "artifact_id": artifact_id,
                        "kind": kind,
                        "title": title,
                        "workspace_dir": workspace_dir,
                        "path": path,
                    })),
                    ..Default::default()
                });
            }
            _ => {}
        }
    }
}

/// Create a **fresh** approval-surface subscription on the **current** tokio runtime.
///
/// Unlike [`register_approval_surface_subscriber`], which is guarded by a process-level
/// [`OnceLock`] and intended for production use, this function subscribes unconditionally
/// and returns the [`SubscriptionHandle`] to the caller.
///
/// The caller **must keep the returned handle alive** for the duration of the subscription.
/// Dropping it aborts the background task and silently stops bridging events.
///
/// Primary use-case: integration tests that spin up a fresh tokio runtime per test.
/// The OnceLock-guarded singleton is tied to the runtime it was first registered on; when
/// that runtime drops, the task is cancelled and subsequent tests in the same process can no
/// longer receive `approval_request` SSE events. Calling this function once per test and
/// storing the handle on a local variable ensures the bridge runs on — and lives for exactly
/// as long as — the current test's runtime.
///
/// Compiled only in debug builds (`#[cfg(debug_assertions)]`) so this OnceLock-bypassing
/// helper can never be linked into a release binary, where a second live subscriber would
/// surface every `ApprovalRequested` event twice. Production always uses the singleton
/// [`register_approval_surface_subscriber`].
#[cfg(debug_assertions)]
#[doc(hidden)]
pub fn fresh_approval_surface_subscription() -> Option<SubscriptionHandle> {
    tracing::trace!(
        "[web-channel] fresh_approval_surface_subscription — debug-only OnceLock bypass, \
         registering a per-runtime approval-surface bridge for tests"
    );
    crate::core::bus::BUS.subscribe(Arc::new(ApprovalSurfaceSubscriber))
}

/// Build the `approval_request` web-channel event for a parked approval.
///
/// Shared by the live surface below (on `ApprovalRequested`) and by the replay
/// path in `openhuman_rpc::server::socketio` (a socket joining a thread room that already has an
/// approval parked on it). One constructor so a client that missed the live
/// emit is handed a byte-identical payload rather than a second shape kept in
/// step by hand.
pub fn approval_request_event(
    request_id: &str,
    tool_name: &str,
    action_summary: &str,
    args_redacted: &serde_json::Value,
    thread_id: &str,
    client_id: &str,
    tool_call_id: Option<&str>,
    expires_at: Option<&str>,
    detached: bool,
) -> WebChannelEvent {
    WebChannelEvent {
        event: "approval_request".to_string(),
        client_id: client_id.to_string(),
        thread_id: thread_id.to_string(),
        request_id: request_id.to_string(),
        tool_name: Some(tool_name.to_string()),
        message: Some(format!("Run `{tool_name}` — {action_summary}")),
        args: Some(args_redacted.clone()),
        tool_call_id: tool_call_id.map(str::to_string),
        expires_at: expires_at.map(str::to_string),
        detached: detached.then_some(true),
        ..Default::default()
    }
}

/// Build the `plan_review_request` web-channel event for a parked plan
/// review. Shared by the live surface below (on `PlanReviewRequested`) and by
/// the replay path in `openhuman_rpc::server::socketio` (a socket joining a thread room that
/// already has a review parked on it) — one constructor so a client that
/// missed the live emit is handed a byte-identical payload.
pub fn plan_review_request_event(
    request_id: &str,
    summary: &str,
    steps: &[String],
    thread_id: &str,
    client_id: &str,
    tool_call_id: Option<&str>,
    expires_at: Option<&str>,
) -> WebChannelEvent {
    WebChannelEvent {
        event: "plan_review_request".to_string(),
        client_id: client_id.to_string(),
        thread_id: thread_id.to_string(),
        request_id: request_id.to_string(),
        tool_name: Some("request_plan_review".to_string()),
        message: Some(summary.to_string()),
        args: Some(serde_json::json!({ "steps": steps })),
        tool_call_id: tool_call_id.map(str::to_string),
        expires_at: expires_at.map(str::to_string),
        ..Default::default()
    }
}

struct ApprovalSurfaceSubscriber;

#[async_trait]
impl EventHandler<DomainEvent> for ApprovalSurfaceSubscriber {
    fn name(&self) -> &str {
        "web_chat::approval_surface"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["approval", "plan_review"])
    }

    async fn handle(&self, event: &DomainEvent) {
        match event {
            DomainEvent::ApprovalRequested {
                request_id,
                tool_name,
                action_summary,
                args_redacted,
                thread_id,
                client_id,
                tool_call_id,
                expires_at,
                agent_id: _,
            } => match (thread_id, client_id) {
                (Some(thread_id), Some(client_id)) => {
                    log::info!(
                        "[web-channel] approval-surface emitting approval_request request_id={request_id} thread_id={thread_id} client_id={client_id} tool={tool_name}"
                    );
                    publish_web_channel_event(approval_request_event(
                        request_id,
                        tool_name,
                        action_summary,
                        args_redacted,
                        thread_id,
                        client_id,
                        tool_call_id.as_deref(),
                        expires_at.as_deref(),
                        crate::security::approval::is_detached_request(request_id),
                    ));
                }
                _ => {
                    log::warn!(
                        "[web-channel] approval-surface received ApprovalRequested request_id={request_id} tool={tool_name} but thread_id/client_id absent (thread={}, client={}) — NOT surfacing",
                        thread_id.is_some(),
                        client_id.is_some()
                    );
                }
            },
            DomainEvent::ApprovalDecided {
                request_id,
                tool_name,
                decision,
                thread_id,
                client_id,
                tool_call_id,
                resolution,
                agent_id: _,
            } => match (thread_id, client_id) {
                (Some(thread_id), Some(client_id)) => {
                    log::info!(
                        "[web-channel] approval-surface emitting approval_decided request_id={request_id} thread_id={thread_id} client_id={client_id} tool={tool_name} decision={decision}"
                    );
                    publish_web_channel_event(WebChannelEvent {
                        event: "approval_decided".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
                        tool_name: Some(tool_name.clone()),
                        message: Some(decision.clone()),
                        tool_call_id: tool_call_id.clone(),
                        cancel_reason: resolution.clone(),
                        ..Default::default()
                    });
                }
                _ => {
                    log::debug!(
                        "[web-channel] approval-surface received ApprovalDecided request_id={request_id} tool={tool_name} decision={decision} but thread_id/client_id absent — NOT surfacing (non-chat origin)"
                    );
                }
            },
            DomainEvent::PlanReviewRequested {
                request_id,
                thread_id,
                client_id,
                summary,
                steps,
                tool_call_id,
                expires_at,
            } => match (thread_id, client_id) {
                (Some(thread_id), Some(client_id)) => {
                    log::info!(
                        "[web-channel] plan-review-surface emitting plan_review_request request_id={request_id} thread_id={thread_id} client_id={client_id} steps={}",
                        steps.len()
                    );
                    publish_web_channel_event(plan_review_request_event(
                        request_id,
                        summary,
                        steps,
                        thread_id,
                        client_id,
                        tool_call_id.as_deref(),
                        expires_at.as_deref(),
                    ));
                }
                _ => {
                    log::warn!(
                        "[web-channel] plan-review-surface received PlanReviewRequested request_id={request_id} but thread_id/client_id absent (thread={}, client={}) — NOT surfacing",
                        thread_id.is_some(),
                        client_id.is_some()
                    );
                }
            },
            DomainEvent::PlanReviewDecided {
                request_id,
                decision,
                thread_id,
                client_id,
                tool_call_id,
                resolution,
            } => match (thread_id, client_id) {
                (Some(thread_id), Some(client_id)) => {
                    log::info!(
                        "[web-channel] plan-review-surface emitting plan_review_decided request_id={request_id} thread_id={thread_id} client_id={client_id} decision={decision}"
                    );
                    publish_web_channel_event(WebChannelEvent {
                        event: "plan_review_decided".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
                        tool_name: Some("request_plan_review".to_string()),
                        message: Some(decision.clone()),
                        tool_call_id: tool_call_id.clone(),
                        cancel_reason: resolution.clone(),
                        ..Default::default()
                    });
                }
                _ => {
                    log::debug!(
                        "[web-channel] plan-review-surface received PlanReviewDecided request_id={request_id} decision={decision} but thread_id/client_id absent — NOT surfacing (non-chat origin)"
                    );
                }
            },
            _ => {}
        }
    }
}

#[cfg(test)]
#[path = "event_bus_tests.rs"]
mod tests;
