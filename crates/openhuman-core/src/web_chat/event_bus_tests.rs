use super::*;
use crate::web_chat::ScopedWebEvent;
use tokio::sync::broadcast;

fn wire(event: &ScopedWebEvent) -> serde_json::Value {
    serde_json::to_value(event).expect("web event serializes")
}

/// `fresh_approval_surface_subscription` returns `Some` when the global event bus has
/// been initialised and `None` otherwise (bus not started).  It must never return `None`
/// after `init_global` has been called — the production path always initialises the bus
/// before the web channel starts handling requests.
#[tokio::test]
async fn fresh_approval_surface_subscription_returns_some_when_bus_is_ready() {
    crate::core::bus::init().await.expect("bus init");
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
    crate::core::bus::init().await.expect("bus init");
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
    rx: &mut broadcast::Receiver<ScopedWebEvent>,
    marker: &str,
) -> ScopedWebEvent {
    loop {
        match rx.recv().await {
            Ok(ev) if ev.event_name() == "external_transfer_pending" => {
                if wire(&ev)["args"]["service"] == marker {
                    return ev;
                }
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
    crate::core::bus::init().await.expect("bus init");
    let _handle = crate::core::bus::BUS.subscribe(Arc::new(
        crate::web_chat::egress_surface::EgressSurfaceSubscriber,
    ));
    let mut web_rx = subscribe_web_channel_events();

    let marker = "svc-bridge-with-context";
    crate::core::bus::BUS.publish(DomainEvent::ExternalTransferPending {
        descriptor: crate::security::egress::EgressDescriptor::composio(marker),
        thread_id: Some("thread-1".to_string()),
        client_id: Some("client-1".to_string()),
        request_id: Some("request-1".to_string()),
    });

    let ev = find_egress_web_event(&mut web_rx, marker).await;
    assert_eq!(ev.context().thread_id, "thread-1");
    assert_eq!(ev.context().client_id, "client-1");
    assert_eq!(ev.context().request_id, "request-1");
    let args = wire(&ev)["args"].clone();
    assert_eq!(args["provider_slug"], "composio");
    assert_eq!(args["reason"], "tool_call");
    assert_eq!(args["is_external"], true);
}

/// A pending event with no chat routing is NOT surfaced to the web channel
/// (background/CLI/cron egress has no client to fan out to).
#[tokio::test]
async fn egress_surface_drops_pending_without_chat_context() {
    crate::core::bus::init().await.expect("bus init");
    let _handle = crate::core::bus::BUS.subscribe(Arc::new(
        crate::web_chat::egress_surface::EgressSurfaceSubscriber,
    ));
    let mut web_rx = subscribe_web_channel_events();

    let dropped_marker = "svc-bridge-no-context";
    let sentinel_marker = "svc-bridge-sentinel";
    // No context → must be dropped. A following event WITH context must be
    // surfaced; reaching the sentinel proves the first was suppressed.
    crate::core::bus::BUS.publish(DomainEvent::ExternalTransferPending {
        descriptor: crate::security::egress::EgressDescriptor::composio(dropped_marker),
        thread_id: None,
        client_id: None,
        request_id: None,
    });
    crate::core::bus::BUS.publish(DomainEvent::ExternalTransferPending {
        descriptor: crate::security::egress::EgressDescriptor::composio(sentinel_marker),
        thread_id: Some("thread-2".to_string()),
        client_id: Some("client-2".to_string()),
        request_id: None,
    });

    loop {
        match web_rx.recv().await {
            Ok(ev) if ev.event_name() == "external_transfer_pending" => {
                let value = wire(&ev);
                let svc = value["args"]["service"].as_str();
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

/// Drain the web-channel receiver until an event of the given name whose
/// `args.artifact_id` matches `marker` arrives.
async fn find_artifact_web_event(
    rx: &mut broadcast::Receiver<ScopedWebEvent>,
    event_name: &str,
    marker: &str,
) -> ScopedWebEvent {
    loop {
        match rx.recv().await {
            Ok(ev)
                if ev.event_name() == event_name
                    && wire(&ev)["args"]["artifact_id"].as_str() == Some(marker) =>
            {
                return ev;
            }
            Ok(_) => continue,
            Err(broadcast::error::RecvError::Lagged(_)) => continue,
            Err(broadcast::error::RecvError::Closed) => {
                panic!("web-channel bus closed before {event_name} arrived")
            }
        }
    }
}

/// `ArtifactPending`/`Ready`/`Failed` carry `tool_call_id`/`request_id`
/// (C5, correlating a generated artifact card with the tool-call bubble
/// that produced it). The artifact-surface subscriber must bridge both onto
/// `WebChannelEvent.tool_call_id` / `.turn_request_id` for every one of the
/// three lifecycle events.
#[tokio::test]
async fn artifact_surface_bridges_tool_call_id_and_request_id() {
    crate::core::bus::init().await.expect("bus init");
    let _handle = crate::core::bus::BUS.subscribe(Arc::new(ArtifactSurfaceSubscriber));
    let mut web_rx = subscribe_web_channel_events();

    let pending_id = "artifact-corr-pending";
    crate::core::bus::BUS.publish(DomainEvent::ArtifactPending {
        artifact_id: pending_id.to_string(),
        kind: "image".to_string(),
        title: "A cat".to_string(),
        workspace_dir: "/tmp/ws".to_string(),
        path: format!("{pending_id}/a-cat.png"),
        thread_id: Some("thread-1".to_string()),
        client_id: Some("client-1".to_string()),
        tool_call_id: Some("call-1".to_string()),
        request_id: Some("req-1".to_string()),
    });
    let ev = find_artifact_web_event(&mut web_rx, "artifact_pending", pending_id).await;
    assert_eq!(wire(&ev)["tool_call_id"], "call-1");
    assert_eq!(wire(&ev)["turn_request_id"], "req-1");
    assert_eq!(
        wire(&ev)["args"],
        serde_json::json!({
            "artifact_id": pending_id,
            "kind": "image",
            "title": "A cat",
            "workspace_dir": "/tmp/ws",
            "path": "artifact-corr-pending/a-cat.png",
        })
    );

    let ready_id = "artifact-corr-ready";
    crate::core::bus::BUS.publish(DomainEvent::ArtifactReady {
        artifact_id: ready_id.to_string(),
        kind: "image".to_string(),
        title: "A cat".to_string(),
        workspace_dir: "/tmp/ws".to_string(),
        path: format!("{ready_id}/a-cat.png"),
        size_bytes: 42,
        thread_id: Some("thread-1".to_string()),
        client_id: Some("client-1".to_string()),
        tool_call_id: Some("call-2".to_string()),
        request_id: Some("req-2".to_string()),
    });
    let ev = find_artifact_web_event(&mut web_rx, "artifact_ready", ready_id).await;
    assert_eq!(wire(&ev)["tool_call_id"], "call-2");
    assert_eq!(wire(&ev)["turn_request_id"], "req-2");
    assert_eq!(
        wire(&ev)["args"],
        serde_json::json!({
            "artifact_id": ready_id,
            "kind": "image",
            "title": "A cat",
            "workspace_dir": "/tmp/ws",
            "path": "artifact-corr-ready/a-cat.png",
            "size_bytes": 42,
        })
    );

    let failed_id = "artifact-corr-failed";
    crate::core::bus::BUS.publish(DomainEvent::ArtifactFailed {
        artifact_id: failed_id.to_string(),
        kind: "image".to_string(),
        title: "A cat".to_string(),
        workspace_dir: "/tmp/ws".to_string(),
        error: "provider timeout".to_string(),
        thread_id: Some("thread-1".to_string()),
        client_id: Some("client-1".to_string()),
        tool_call_id: Some("call-3".to_string()),
        request_id: Some("req-3".to_string()),
    });
    let ev = find_artifact_web_event(&mut web_rx, "artifact_failed", failed_id).await;
    assert_eq!(wire(&ev)["tool_call_id"], "call-3");
    assert_eq!(wire(&ev)["turn_request_id"], "req-3");
    assert_eq!(
        wire(&ev)["args"],
        serde_json::json!({
            "artifact_id": failed_id,
            "kind": "image",
            "title": "A cat",
            "workspace_dir": "/tmp/ws",
            "error": "provider timeout",
        })
    );
}

#[tokio::test]
async fn artifact_surface_preserves_unknown_kind_wire_value() {
    crate::core::bus::init().await.expect("bus init");
    let _handle = crate::core::bus::BUS.subscribe(Arc::new(ArtifactSurfaceSubscriber));
    let mut web_rx = subscribe_web_channel_events();
    let artifact_id = "artifact-future-kind";

    crate::core::bus::BUS.publish(DomainEvent::ArtifactReady {
        artifact_id: artifact_id.to_string(),
        kind: "future_kind".to_string(),
        title: "A future artifact".to_string(),
        workspace_dir: "/tmp/ws".to_string(),
        path: "future.bin".to_string(),
        size_bytes: 7,
        thread_id: Some("thread-1".to_string()),
        client_id: Some("client-1".to_string()),
        tool_call_id: None,
        request_id: None,
    });

    let event = find_artifact_web_event(&mut web_rx, "artifact_ready", artifact_id).await;
    assert_eq!(wire(&event)["args"]["kind"], "future_kind");
}

/// A producer that ran outside a harness tool-call context (CLI, cron)
/// carries `tool_call_id: None` — the bridged event must not fabricate one.
#[tokio::test]
async fn artifact_surface_leaves_tool_call_id_none_when_absent() {
    crate::core::bus::init().await.expect("bus init");
    let _handle = crate::core::bus::BUS.subscribe(Arc::new(ArtifactSurfaceSubscriber));
    let mut web_rx = subscribe_web_channel_events();

    let ready_id = "artifact-corr-no-call-id";
    crate::core::bus::BUS.publish(DomainEvent::ArtifactReady {
        artifact_id: ready_id.to_string(),
        kind: "document".to_string(),
        title: "A doc".to_string(),
        workspace_dir: "/tmp/ws".to_string(),
        path: format!("{ready_id}/a-doc.docx"),
        size_bytes: 7,
        thread_id: Some("thread-1".to_string()),
        client_id: Some("client-1".to_string()),
        tool_call_id: None,
        request_id: None,
    });
    let ev = find_artifact_web_event(&mut web_rx, "artifact_ready", ready_id).await;
    assert!(wire(&ev).get("tool_call_id").is_none());
    assert!(wire(&ev).get("turn_request_id").is_none());
}

/// Drain the web-channel receiver until an event with the given `event` name
/// and `thread_id` arrives (the bus is process-wide, so unrelated events from
/// other tests may interleave).
pub(super) async fn find_agent_web_event(
    rx: &mut broadcast::Receiver<ScopedWebEvent>,
    event: &str,
    thread_id: &str,
) -> ScopedWebEvent {
    loop {
        match rx.recv().await {
            Ok(ev) if ev.event_name() == event && ev.context().thread_id == thread_id => return ev,
            Ok(_) => continue,
            Err(broadcast::error::RecvError::Lagged(_)) => continue,
            Err(broadcast::error::RecvError::Closed) => {
                panic!("web-channel bus closed before {event} arrived")
            }
        }
    }
}

/// `ThreadGoalUpdated` bridges to `thread_goal_updated` carrying the full
/// goal payload, with an empty `client_id` (goal, todo, and queue events are
/// thread-scoped, not client-scoped — see `AgentSurfaceSubscriber`'s docs).
#[tokio::test]
async fn agent_surface_bridges_thread_goal_updated() {
    crate::core::bus::init().await.expect("bus init");
    let _handle = crate::core::bus::BUS.subscribe(Arc::new(AgentSurfaceSubscriber));
    let mut web_rx = subscribe_web_channel_events();

    let thread_id = "thread-goal-updated";
    let goal = serde_json::json!({ "objective": "ship it", "status": "active" });
    crate::core::bus::BUS.publish(DomainEvent::ThreadGoalUpdated {
        thread_id: thread_id.to_string(),
        goal_id: "goal-1".to_string(),
        status: "active".to_string(),
        goal: Some(goal.clone()),
    });

    let ev = find_agent_web_event(&mut web_rx, "thread_goal_updated", thread_id).await;
    assert_eq!(ev.context().client_id, "");
    assert_eq!(wire(&ev)["goal"], goal);
}

/// Goal resume and committed-turn accounting use the same observed owner store
/// as model-facing goal tools, so both lifecycle paths reach the existing
/// thread-scoped socket event.
#[tokio::test]
async fn goal_resume_and_account_updates_reach_the_web_channel() {
    crate::core::bus::init().await.expect("bus init");
    register_agent_surface_subscriber();
    let mut web_rx = subscribe_web_channel_events();

    let workspace = tempfile::tempdir().expect("workspace");
    let context = crate::core::runtime::CoreContext::for_test(
        crate::core::runtime::DomainSet::full(),
        Some(workspace.path().to_path_buf()),
    );
    crate::core::runtime::CoreContext::scope(context, async {
        let dispatch = crate::core::runtime::DispatchCtx::ambient();
        let store = crate::agent::goals::goal_store(workspace.path(), &dispatch);
        let thread_id = "goal-observer-resume-account";

        store
            .set(thread_id, "resume this goal", Some(100))
            .await
            .expect("set goal");
        let _ = find_agent_web_event(&mut web_rx, "thread_goal_updated", thread_id).await;
        store.pause(thread_id).await.expect("pause goal");
        let _ = find_agent_web_event(&mut web_rx, "thread_goal_updated", thread_id).await;

        let prepared = store.prepare_turn(thread_id).await.expect("prepare turn");
        let paused = tinyagents_harness_tools::goals::ThreadGoalStatus::Paused;
        assert_eq!(prepared.goal.map(|goal| goal.status), Some(paused));
        store.resume(thread_id).await.expect("resume goal");
        let resumed = find_agent_web_event(&mut web_rx, "thread_goal_updated", thread_id).await;
        assert_eq!(
            wire(&resumed)["goal"]["status"],
            serde_json::json!("active")
        );

        crate::agent::goals::account_turn(
            workspace.path(),
            &dispatch,
            Some(thread_id),
            2,
            0,
            std::time::Duration::from_secs(1),
        )
        .await;
        drop(store);
        crate::agent::goals::account_turn(
            workspace.path(),
            &dispatch,
            Some(thread_id),
            2,
            0,
            std::time::Duration::from_secs(1),
        )
        .await;
        loop {
            match web_rx.try_recv() {
                Ok(event)
                    if event.event_name() == "thread_goal_updated"
                        && event.context().thread_id == thread_id =>
                {
                    panic!("account update published before the throttle threshold")
                }
                Ok(_) | Err(broadcast::error::TryRecvError::Lagged(_)) => continue,
                Err(broadcast::error::TryRecvError::Empty) => break,
                Err(broadcast::error::TryRecvError::Closed) => {
                    panic!("web-channel bus closed before the account threshold")
                }
            }
        }
        crate::agent::goals::account_turn(
            workspace.path(),
            &dispatch,
            Some(thread_id),
            1,
            0,
            std::time::Duration::from_secs(1),
        )
        .await;
        let accounted = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            find_agent_web_event(&mut web_rx, "thread_goal_updated", thread_id),
        )
        .await
        .expect("throttled account event arrives after the threshold");
        assert_eq!(wire(&accounted)["goal"]["tokensUsed"], serde_json::json!(5));
    })
    .await;
}
