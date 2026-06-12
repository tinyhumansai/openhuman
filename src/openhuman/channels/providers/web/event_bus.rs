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
                "[web-channel] approval-surface subscriber registered (domain=approval) — will bridge ApprovalRequested → approval_request socket event"
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

struct ApprovalSurfaceSubscriber;

#[async_trait]
impl EventHandler for ApprovalSurfaceSubscriber {
    fn name(&self) -> &str {
        "channels::web::approval_surface"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["approval"])
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
        }
    }
}
