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

static EGRESS_SURFACE_HANDLE: OnceLock<SubscriptionHandle> = OnceLock::new();

/// Register the egress-surface bridge that turns
/// [`DomainEvent::ExternalTransferPending`] events into
/// `external_transfer_pending` web-channel socket events (privacy epic S2,
/// #4436). Idempotent via a process-level [`OnceLock`].
pub fn register_egress_surface_subscriber() {
    if EGRESS_SURFACE_HANDLE.get().is_some() {
        return;
    }
    match crate::core::event_bus::subscribe_global(Arc::new(EgressSurfaceSubscriber)) {
        Some(handle) => {
            let _ = EGRESS_SURFACE_HANDLE.set(handle);
            log::info!(
                "[web-channel] egress-surface subscriber registered (domain=egress) — bridges ExternalTransferPending → external_transfer_pending socket events"
            );
        }
        None => {
            log::warn!(
                "[web-channel] failed to register egress-surface subscriber — bus not initialized"
            );
        }
    }
}

/// Bridge [`DomainEvent::ExternalTransferPending`] → `external_transfer_pending`
/// web-channel socket event so the frontend can disclose the transfer (S3
/// renders the card; S4 will add an approve/deny arm). Only surfaces transfers
/// that carry chat routing — background/CLI/cron egress has no chat client to
/// fan out to and is dropped here (still observable on the domain bus for
/// non-chat consumers such as an audit log).
struct EgressSurfaceSubscriber;

#[async_trait]
impl EventHandler for EgressSurfaceSubscriber {
    fn name(&self) -> &str {
        "web_chat::egress_surface"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["egress"])
    }

    async fn handle(&self, event: &DomainEvent) {
        let DomainEvent::ExternalTransferPending {
            descriptor,
            thread_id,
            client_id,
        } = event
        else {
            return;
        };
        let (Some(thread_id), Some(client_id)) = (thread_id, client_id) else {
            log::debug!(
                "[web-channel] egress-surface skip ExternalTransferPending provider={} service={} reason={:?}: no chat context",
                descriptor.provider_slug,
                descriptor.service,
                descriptor.reason,
            );
            return;
        };
        let args = match serde_json::to_value(descriptor) {
            Ok(value) => value,
            Err(e) => {
                log::warn!(
                    "[web-channel] egress-surface failed to serialize descriptor provider={} service={}: {e}",
                    descriptor.provider_slug,
                    descriptor.service,
                );
                return;
            }
        };
        log::info!(
            "[web-channel] egress-surface emitting external_transfer_pending provider={} service={} reason={:?} thread_id={thread_id} client_id={client_id}",
            descriptor.provider_slug,
            descriptor.service,
            descriptor.reason,
        );
        publish_web_channel_event(WebChannelEvent {
            event: "external_transfer_pending".to_string(),
            client_id: client_id.clone(),
            thread_id: thread_id.clone(),
            args: Some(args),
            ..Default::default()
        });
    }
}

struct ArtifactSurfaceSubscriber;

#[async_trait]
impl EventHandler for ArtifactSurfaceSubscriber {
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
        "web_chat::approval_surface"
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

    /// Drain the web-channel receiver until an `external_transfer_pending` event
    /// whose `args.service` matches `marker` arrives (the bus is process-wide).
    async fn find_egress_web_event(
        rx: &mut broadcast::Receiver<WebChannelEvent>,
        marker: &str,
    ) -> WebChannelEvent {
        loop {
            match rx.recv().await {
                Ok(ev)
                    if ev.event == "external_transfer_pending"
                        && ev
                            .args
                            .as_ref()
                            .and_then(|a| a.get("service"))
                            .and_then(|s| s.as_str())
                            == Some(marker) =>
                {
                    return ev
                }
                Ok(_) => continue,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => {
                    panic!("web-channel bus closed before external_transfer_pending arrived")
                }
            }
        }
    }

    /// Egress-surface bridges an `ExternalTransferPending` that carries chat
    /// routing into an `external_transfer_pending` web-channel event whose args
    /// mirror the descriptor (privacy epic S2, #4436).
    #[tokio::test]
    async fn egress_surface_bridges_pending_with_chat_context() {
        crate::core::event_bus::init_global(crate::core::event_bus::DEFAULT_CAPACITY);
        let _handle = crate::core::event_bus::subscribe_global(Arc::new(EgressSurfaceSubscriber));
        let mut web_rx = subscribe_web_channel_events();

        let marker = "svc-bridge-with-context";
        crate::core::event_bus::publish_global(DomainEvent::ExternalTransferPending {
            descriptor: crate::openhuman::security::egress::EgressDescriptor::composio(marker),
            thread_id: Some("thread-1".to_string()),
            client_id: Some("client-1".to_string()),
        });

        let ev = find_egress_web_event(&mut web_rx, marker).await;
        assert_eq!(ev.thread_id, "thread-1");
        assert_eq!(ev.client_id, "client-1");
        let args = ev.args.expect("args present");
        assert_eq!(args["provider_slug"], "composio");
        assert_eq!(args["reason"], "tool_call");
        assert_eq!(args["is_external"], true);
    }

    /// A pending event with no chat routing is NOT surfaced to the web channel
    /// (background/CLI/cron egress has no client to fan out to).
    #[tokio::test]
    async fn egress_surface_drops_pending_without_chat_context() {
        crate::core::event_bus::init_global(crate::core::event_bus::DEFAULT_CAPACITY);
        let _handle = crate::core::event_bus::subscribe_global(Arc::new(EgressSurfaceSubscriber));
        let mut web_rx = subscribe_web_channel_events();

        let dropped_marker = "svc-bridge-no-context";
        let sentinel_marker = "svc-bridge-sentinel";
        // No context → must be dropped. A following event WITH context must be
        // surfaced; reaching the sentinel proves the first was suppressed.
        crate::core::event_bus::publish_global(DomainEvent::ExternalTransferPending {
            descriptor: crate::openhuman::security::egress::EgressDescriptor::composio(
                dropped_marker,
            ),
            thread_id: None,
            client_id: None,
        });
        crate::core::event_bus::publish_global(DomainEvent::ExternalTransferPending {
            descriptor: crate::openhuman::security::egress::EgressDescriptor::composio(
                sentinel_marker,
            ),
            thread_id: Some("thread-2".to_string()),
            client_id: Some("client-2".to_string()),
        });

        loop {
            match web_rx.recv().await {
                Ok(ev) if ev.event == "external_transfer_pending" => {
                    let svc = ev
                        .args
                        .as_ref()
                        .and_then(|a| a.get("service"))
                        .and_then(|s| s.as_str());
                    assert_ne!(
                        svc,
                        Some(dropped_marker),
                        "no-context transfer must not surface to the web channel"
                    );
                    if svc == Some(sentinel_marker) {
                        break;
                    }
                }
                Ok(_) => continue,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => {
                    panic!("web-channel bus closed before sentinel arrived")
                }
            }
        }
    }
}
