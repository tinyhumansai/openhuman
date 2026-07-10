use async_trait::async_trait;
use once_cell::sync::Lazy;
use std::sync::{Arc, OnceLock};
use tokio::sync::broadcast;

use crate::core::event_bus::{DomainEvent, EventHandler, SubscriptionHandle};
use crate::core::socketio::WebChannelEvent;

static EVENT_BUS: Lazy<broadcast::Sender<WebChannelEvent>> = Lazy::new(|| {
    let (tx, _rx) = broadcast::channel(512);
    tx
});

pub fn subscribe_web_channel_events() -> broadcast::Receiver<WebChannelEvent> {
    EVENT_BUS.subscribe()
}

pub fn publish_web_channel_event(event: WebChannelEvent) {
    let _ = EVENT_BUS.send(event);
}

static APPROVAL_SURFACE_HANDLE: OnceLock<SubscriptionHandle> = OnceLock::new();

pub fn register_approval_surface_subscriber() {
    if APPROVAL_SURFACE_HANDLE.get().is_some() {
        return;
    }
    match crate::core::event_bus::subscribe_global(Arc::new(ApprovalSurfaceSubscriber)) {
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
    match crate::core::event_bus::subscribe_global(Arc::new(ArtifactSurfaceSubscriber)) {
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

struct ArtifactSurfaceSubscriber;

#[async_trait]
impl EventHandler for ArtifactSurfaceSubscriber {
    fn name(&self) -> &str {
        "channels::web::artifact_surface"
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
            } => {
                let (Some(thread_id), Some(client_id)) = (thread_id, client_id) else {
                    log::debug!(
                        "[web-channel] artifact-surface skip ArtifactReady id={artifact_id}: no chat context"
                    );
                    return;
                };
                log::info!(
                    "[web-channel] artifact-surface emitting artifact_ready id={artifact_id} kind={kind} thread_id={thread_id} client_id={client_id}"
                );
                publish_web_channel_event(WebChannelEvent {
                    event: "artifact_ready".to_string(),
                    client_id: client_id.clone(),
                    thread_id: thread_id.clone(),
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
            } => {
                let (Some(thread_id), Some(client_id)) = (thread_id, client_id) else {
                    log::debug!(
                        "[web-channel] artifact-surface skip ArtifactFailed id={artifact_id}: no chat context"
                    );
                    return;
                };
                log::warn!(
                    "[web-channel] artifact-surface emitting artifact_failed id={artifact_id} kind={kind} thread_id={thread_id} client_id={client_id} error_len={}",
                    error.len()
                );
                publish_web_channel_event(WebChannelEvent {
                    event: "artifact_failed".to_string(),
                    client_id: client_id.clone(),
                    thread_id: thread_id.clone(),
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
            } => {
                let (Some(thread_id), Some(client_id)) = (thread_id, client_id) else {
                    log::debug!(
                        "[web-channel] artifact-surface skip ArtifactPending id={artifact_id}: no chat context"
                    );
                    return;
                };
                log::info!(
                    "[web-channel] artifact-surface emitting artifact_pending id={artifact_id} kind={kind} thread_id={thread_id} client_id={client_id}"
                );
                publish_web_channel_event(WebChannelEvent {
                    event: "artifact_pending".to_string(),
                    client_id: client_id.clone(),
                    thread_id: thread_id.clone(),
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
    crate::core::event_bus::subscribe_global(Arc::new(ApprovalSurfaceSubscriber))
}

struct ApprovalSurfaceSubscriber;

#[async_trait]
impl EventHandler for ApprovalSurfaceSubscriber {
    fn name(&self) -> &str {
        "channels::web::approval_surface"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["approval", "plan_review"])
    }

    async fn handle(&self, event: &DomainEvent) {
        if let DomainEvent::ApprovalRequested {
            request_id,
            tool_name,
            action_summary,
            args_redacted,
            thread_id,
            client_id,
            ..
        } = event
        {
            match (thread_id, client_id) {
                (Some(thread_id), Some(client_id)) => {
                    let question = format!("Run `{tool_name}` — {action_summary}");
                    log::info!(
                        "[web-channel] approval-surface emitting approval_request request_id={request_id} thread_id={thread_id} client_id={client_id} tool={tool_name}"
                    );
                    publish_web_channel_event(WebChannelEvent {
                        event: "approval_request".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
                        tool_name: Some(tool_name.clone()),
                        message: Some(question),
                        args: Some(args_redacted.clone()),
                        ..Default::default()
                    });
                }
                _ => {
                    log::warn!(
                        "[web-channel] approval-surface received ApprovalRequested request_id={request_id} tool={tool_name} but thread_id/client_id absent (thread={}, client={}) — NOT surfacing",
                        thread_id.is_some(),
                        client_id.is_some()
                    );
                }
            }
        } else if let DomainEvent::PlanReviewRequested {
            request_id,
            thread_id,
            client_id,
            summary,
            steps,
        } = event
        {
            match (thread_id, client_id) {
                (Some(thread_id), Some(client_id)) => {
                    log::info!(
                        "[web-channel] plan-review-surface emitting plan_review_request request_id={request_id} thread_id={thread_id} client_id={client_id} steps={}",
                        steps.len()
                    );
                    publish_web_channel_event(WebChannelEvent {
                        event: "plan_review_request".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
                        tool_name: Some("request_plan_review".to_string()),
                        message: Some(summary.clone()),
                        args: Some(serde_json::json!({ "steps": steps })),
                        ..Default::default()
                    });
                }
                _ => {
                    log::warn!(
                        "[web-channel] plan-review-surface received PlanReviewRequested request_id={request_id} but thread_id/client_id absent (thread={}, client={}) — NOT surfacing",
                        thread_id.is_some(),
                        client_id.is_some()
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `fresh_approval_surface_subscription` returns `Some` when the global event bus has
    /// been initialised and `None` otherwise (bus not started).  It must never return `None`
    /// after `init_global` has been called — the production path always initialises the bus
    /// before the web channel starts handling requests.
    #[tokio::test]
    async fn fresh_approval_surface_subscription_returns_some_when_bus_is_ready() {
        crate::core::event_bus::init_global(crate::core::event_bus::DEFAULT_CAPACITY);
        let handle = fresh_approval_surface_subscription();
        assert!(
            handle.is_some(),
            "fresh_approval_surface_subscription() must return Some when the global event bus \
             is initialised"
        );
    }

    /// Calling `fresh_approval_surface_subscription` multiple times returns independent
    /// handles.  Each is backed by its own background task so multiple callers can bridge
    /// independently (e.g. multiple integration tests running sequentially in the same
    /// process, each on their own tokio runtime).
    #[tokio::test]
    async fn fresh_approval_surface_subscription_is_not_a_singleton() {
        crate::core::event_bus::init_global(crate::core::event_bus::DEFAULT_CAPACITY);
        let h1 = fresh_approval_surface_subscription();
        let h2 = fresh_approval_surface_subscription();
        assert!(h1.is_some(), "first subscription handle must be Some");
        assert!(h2.is_some(), "second subscription handle must be Some");
        // Both handles are alive — drop explicitly to show they're independent.
        drop(h1);
        drop(h2);
    }
}
