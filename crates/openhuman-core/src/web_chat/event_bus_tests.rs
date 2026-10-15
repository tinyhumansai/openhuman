use super::*;
use tokio::sync::broadcast;

/// `fresh_approval_surface_subscription` returns `Some` when the global event bus has
/// been initialised and `None` otherwise (bus not started).  It must never return `None`
/// after `init_global` has been called — the production path always initialises the bus
/// before the web channel starts handling requests.
#[cfg(debug_assertions)]
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
#[cfg(debug_assertions)]
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
                return ev;
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
    assert_eq!(ev.thread_id, "thread-1");
    assert_eq!(ev.client_id, "client-1");
    assert_eq!(ev.request_id, "request-1");
    let args = ev.args.expect("args present");
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

/// Drain the web-channel receiver until an event of the given name whose
/// `args.artifact_id` matches `marker` arrives.
async fn find_artifact_web_event(
    rx: &mut broadcast::Receiver<WebChannelEvent>,
    event_name: &str,
    marker: &str,
) -> WebChannelEvent {
    loop {
        match rx.recv().await {
            Ok(ev)
                if ev.event == event_name
                    && ev
                        .args
                        .as_ref()
                        .and_then(|a| a.get("artifact_id"))
                        .and_then(|s| s.as_str())
                        == Some(marker) =>
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
    assert_eq!(ev.tool_call_id, Some("call-1".to_string()));
    assert_eq!(ev.turn_request_id, Some("req-1".to_string()));

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
    assert_eq!(ev.tool_call_id, Some("call-2".to_string()));
    assert_eq!(ev.turn_request_id, Some("req-2".to_string()));

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
    assert_eq!(ev.tool_call_id, Some("call-3".to_string()));
    assert_eq!(ev.turn_request_id, Some("req-3".to_string()));
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
    assert_eq!(ev.tool_call_id, None);
    assert_eq!(ev.turn_request_id, None);
}

/// Drain the web-channel receiver until an event with the given `event` name
/// and `thread_id` arrives (the bus is process-wide, so unrelated events from
/// other tests may interleave).
async fn find_agent_web_event(
    rx: &mut broadcast::Receiver<WebChannelEvent>,
    event: &str,
    thread_id: &str,
) -> WebChannelEvent {
    loop {
        match rx.recv().await {
            Ok(ev) if ev.event == event && ev.thread_id == thread_id => return ev,
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
    assert_eq!(ev.client_id, "");
    assert_eq!(ev.goal, Some(goal));
}

/// `ThreadGoalCleared` bridges to `thread_goal_cleared`.
#[tokio::test]
async fn agent_surface_bridges_thread_goal_cleared() {
    crate::core::bus::init().await.expect("bus init");
    let _handle = crate::core::bus::BUS.subscribe(Arc::new(AgentSurfaceSubscriber));
    let mut web_rx = subscribe_web_channel_events();

    let thread_id = "thread-goal-cleared";
    crate::core::bus::BUS.publish(DomainEvent::ThreadGoalCleared {
        thread_id: thread_id.to_string(),
    });

    let ev = find_agent_web_event(&mut web_rx, "thread_goal_cleared", thread_id).await;
    assert_eq!(ev.client_id, "");
}

/// `ThreadTodosChanged` bridges to `thread_todos_changed` carrying the todos
/// snapshot.
#[tokio::test]
async fn agent_surface_bridges_thread_todos_changed() {
    crate::core::bus::init().await.expect("bus init");
    let _handle = crate::core::bus::BUS.subscribe(Arc::new(AgentSurfaceSubscriber));
    let mut web_rx = subscribe_web_channel_events();

    let thread_id = "thread-todos-changed";
    let todos = serde_json::json!([{ "content": "write tests", "status": "in_progress" }]);
    crate::core::bus::BUS.publish(DomainEvent::ThreadTodosChanged {
        thread_id: thread_id.to_string(),
        todos: todos.clone(),
    });

    let ev = find_agent_web_event(&mut web_rx, "thread_todos_changed", thread_id).await;
    assert_eq!(ev.todos, Some(todos));
}

/// `RunQueueMessageQueued` bridges to `queue_item_queued` with the item's id
/// and preview, when present.
#[tokio::test]
async fn agent_surface_bridges_queue_item_queued() {
    crate::core::bus::init().await.expect("bus init");
    let _handle = crate::core::bus::BUS.subscribe(Arc::new(AgentSurfaceSubscriber));
    let mut web_rx = subscribe_web_channel_events();

    let thread_id = "thread-queue-queued";
    crate::core::bus::BUS.publish(DomainEvent::RunQueueMessageQueued {
        thread_id: thread_id.to_string(),
        mode: "steer".to_string(),
        queue_depth: 1,
        item_id: Some("item-1".to_string()),
        text_preview: Some("hello".to_string()),
    });

    let ev = find_agent_web_event(&mut web_rx, "queue_item_queued", thread_id).await;
    let item = ev.queue_item.expect("queue_item");
    assert_eq!(item.id, "item-1");
    assert_eq!(item.text_preview, Some("hello".to_string()));
}

/// A `RunQueueMessageQueued` with no `item_id` (not yet minted at the
/// publish site) is not surfaced — the frontend has nothing stable to key on.
#[tokio::test]
async fn agent_surface_skips_queue_item_queued_without_item_id() {
    crate::core::bus::init().await.expect("bus init");
    let _handle = crate::core::bus::BUS.subscribe(Arc::new(AgentSurfaceSubscriber));
    let mut web_rx = subscribe_web_channel_events();

    let thread_id = "thread-queue-queued-no-id";
    crate::core::bus::BUS.publish(DomainEvent::RunQueueMessageQueued {
        thread_id: thread_id.to_string(),
        mode: "steer".to_string(),
        queue_depth: 1,
        item_id: None,
        text_preview: None,
    });
    // Follow with a distinct, surfaced event on the same thread so we can
    // prove the loop reached past the skipped one instead of just timing out.
    crate::core::bus::BUS.publish(DomainEvent::ThreadGoalCleared {
        thread_id: thread_id.to_string(),
    });
    let ev = find_agent_web_event(&mut web_rx, "thread_goal_cleared", thread_id).await;
    assert_eq!(ev.thread_id, thread_id);
}

/// `RunQueueMessageDelivered` bridges to `queue_item_delivered` carrying the
/// lane in `queue_item.lane`.
#[tokio::test]
async fn agent_surface_bridges_queue_item_delivered_with_lane() {
    crate::core::bus::init().await.expect("bus init");
    let _handle = crate::core::bus::BUS.subscribe(Arc::new(AgentSurfaceSubscriber));
    let mut web_rx = subscribe_web_channel_events();

    let thread_id = "thread-queue-delivered";
    crate::core::bus::BUS.publish(DomainEvent::RunQueueMessageDelivered {
        thread_id: thread_id.to_string(),
        mode: "collect".to_string(),
        delivered: 1,
        item_id: Some("item-2".to_string()),
        text_preview: Some("context line".to_string()),
    });

    let ev = find_agent_web_event(&mut web_rx, "queue_item_delivered", thread_id).await;
    let item = ev.queue_item.expect("queue_item");
    assert_eq!(item.id, "item-2");
    assert_eq!(item.lane, Some("collect".to_string()));
}

/// `ThreadRunModeChanged` bridges to `run_mode_changed` with the mode label
/// carried on `message` and an empty `client_id` (thread-scoped, not
/// client-scoped, like the goal/todo/queue events above).
#[tokio::test]
async fn agent_surface_bridges_run_mode_changed() {
    crate::core::bus::init().await.expect("bus init");
    let _handle = crate::core::bus::BUS.subscribe(Arc::new(AgentSurfaceSubscriber));
    let mut web_rx = subscribe_web_channel_events();

    let thread_id = "thread-run-mode-changed";
    crate::core::bus::BUS.publish(DomainEvent::ThreadRunModeChanged {
        thread_id: thread_id.to_string(),
        mode: "plan".to_string(),
    });

    let ev = find_agent_web_event(&mut web_rx, "run_mode_changed", thread_id).await;
    assert_eq!(ev.client_id, "");
    assert_eq!(ev.message, Some("plan".to_string()));
}

/// `ApprovalDecided` with both `thread_id`/`client_id` set bridges to
/// `approval_decided`, mirroring `tool_call_id` and carrying `resolution` on
/// `cancel_reason`.
#[tokio::test]
async fn approval_surface_bridges_approval_decided_with_resolution() {
    crate::core::bus::init().await.expect("bus init");
    let _handle = crate::core::bus::BUS.subscribe(Arc::new(ApprovalSurfaceSubscriber));
    let mut web_rx = subscribe_web_channel_events();

    crate::core::bus::BUS.publish(DomainEvent::ApprovalDecided {
        request_id: "req-decided-1".to_string(),
        tool_name: "composio".to_string(),
        decision: "deny".to_string(),
        thread_id: Some("thread-decided-1".to_string()),
        client_id: Some("client-decided-1".to_string()),
        tool_call_id: Some("call-decided-1".to_string()),
        resolution: Some("expired".to_string()),
        agent_id: None,
    });

    let ev = find_agent_web_event(&mut web_rx, "approval_decided", "thread-decided-1").await;
    assert_eq!(ev.client_id, "client-decided-1");
    assert_eq!(ev.request_id, "req-decided-1");
    assert_eq!(ev.tool_call_id, Some("call-decided-1".to_string()));
    assert_eq!(ev.cancel_reason, Some("expired".to_string()));
    assert_eq!(ev.message, Some("deny".to_string()));
}

/// `ApprovalDecided` with no thread/client routing (a non-chat origin) is
/// intentionally NOT surfaced — there is no room to deliver it to.
#[tokio::test]
async fn approval_surface_drops_approval_decided_without_chat_routing() {
    crate::core::bus::init().await.expect("bus init");
    let _handle = crate::core::bus::BUS.subscribe(Arc::new(ApprovalSurfaceSubscriber));
    let mut web_rx = subscribe_web_channel_events();

    crate::core::bus::BUS.publish(DomainEvent::ApprovalDecided {
        request_id: "req-decided-no-route".to_string(),
        tool_name: "composio".to_string(),
        decision: "deny".to_string(),
        thread_id: None,
        client_id: None,
        tool_call_id: None,
        resolution: Some("expired".to_string()),
        agent_id: None,
    });
    // A sibling event we know fires, so we don't just race an empty channel.
    crate::core::bus::BUS.publish(DomainEvent::ThreadGoalCleared {
        thread_id: "thread-decided-no-route-sentinel".to_string(),
    });

    // `ThreadGoalCleared` isn't in ApprovalSurfaceSubscriber's domain filter,
    // so use a short bounded wait instead: if `approval_decided` were going
    // to arrive, it would arrive well within this window.
    let outcome = tokio::time::timeout(std::time::Duration::from_millis(200), async {
        loop {
            match web_rx.recv().await {
                Ok(ev)
                    if ev.event == "approval_decided"
                        && ev.request_id == "req-decided-no-route" =>
                {
                    return Some(ev)
                }
                Ok(_) => continue,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    })
    .await;
    assert!(
        outcome.is_err(),
        "approval_decided must not surface without thread_id/client_id routing"
    );
}

/// `PlanReviewDecided` bridges to `plan_review_decided`.
#[tokio::test]
async fn plan_review_surface_bridges_plan_review_decided() {
    crate::core::bus::init().await.expect("bus init");
    let _handle = crate::core::bus::BUS.subscribe(Arc::new(ApprovalSurfaceSubscriber));
    let mut web_rx = subscribe_web_channel_events();

    crate::core::bus::BUS.publish(DomainEvent::PlanReviewDecided {
        request_id: "plan-decided-1".to_string(),
        decision: "approve".to_string(),
        thread_id: Some("thread-plan-decided-1".to_string()),
        client_id: Some("client-plan-decided-1".to_string()),
        tool_call_id: Some("call-plan-decided-1".to_string()),
        resolution: None,
    });

    let ev =
        find_agent_web_event(&mut web_rx, "plan_review_decided", "thread-plan-decided-1").await;
    assert_eq!(ev.client_id, "client-plan-decided-1");
    assert_eq!(ev.tool_call_id, Some("call-plan-decided-1".to_string()));
    assert_eq!(ev.cancel_reason, None);
    assert_eq!(ev.message, Some("approve".to_string()));
}

/// `publish_web_channel_event` stamps `ts` (epoch ms) when the caller left it
/// unset, so every emitted event carries a wall-clock time even when the
/// producer never set one explicitly.
#[tokio::test]
async fn publish_web_channel_event_stamps_ts_when_unset() {
    let mut web_rx = subscribe_web_channel_events();
    let before = crate::web_chat::progress_bridge::unix_epoch_ms();

    publish_web_channel_event(WebChannelEvent {
        event: "ts_stamp_probe".to_string(),
        thread_id: "thread-ts-stamp-probe".to_string(),
        ..Default::default()
    });

    let ev = find_agent_web_event(&mut web_rx, "ts_stamp_probe", "thread-ts-stamp-probe").await;
    let after = crate::web_chat::progress_bridge::unix_epoch_ms();
    let ts = ev
        .ts
        .expect("publish_web_channel_event must stamp ts when unset");
    assert!(
        ts >= before && ts <= after,
        "stamped ts ({ts}) must fall within [{before}, {after}]"
    );
}

/// A caller that already set `ts` keeps its own value — `publish_web_channel_event`
/// only fills the field in when it is `None`, so a replayed event (e.g. the
/// parked-approval replay path) keeps its original timestamp instead of being
/// re-stamped with "now".
#[tokio::test]
async fn publish_web_channel_event_preserves_an_explicit_ts() {
    let mut web_rx = subscribe_web_channel_events();

    publish_web_channel_event(WebChannelEvent {
        event: "ts_stamp_probe_explicit".to_string(),
        thread_id: "thread-ts-stamp-probe-explicit".to_string(),
        ts: Some(123),
        ..Default::default()
    });

    let ev = find_agent_web_event(
        &mut web_rx,
        "ts_stamp_probe_explicit",
        "thread-ts-stamp-probe-explicit",
    )
    .await;
    assert_eq!(ev.ts, Some(123));
}

/// A SaaS user's stream carries only events stamped with that user's agent:
/// `publish_web_channel_event` stamps the publishing scope's agent, and an
/// event published outside any agent scope belongs to no user.
#[tokio::test]
async fn published_events_carry_the_publishing_agent() {
    use crate::core::runtime::{ContextOverlay, CoreContext, DomainSet};
    let mut rx = subscribe_web_channel_events();
    let marker = format!("agent-stamp-{}", uuid::Uuid::new_v4());
    let ctx = CoreContext::for_test(DomainSet::full(), None).derive_with(
        ContextOverlay::new(
            crate::config::Config::default(),
            DomainSet::full(),
            Default::default(),
        )
        .session_agent("u-alice")
        .profile("p-alice"),
    );
    CoreContext::scope(ctx, async {
        publish_web_channel_event(WebChannelEvent {
            event: "chat_done".into(),
            thread_id: marker.clone(),
            ..Default::default()
        });
    })
    .await;
    publish_web_channel_event(WebChannelEvent {
        event: "chat_done".into(),
        thread_id: format!("{marker}-unscoped"),
        ..Default::default()
    });

    let mut seen = Vec::new();
    while seen.len() < 2 {
        let ev = rx.recv().await.unwrap();
        if ev.thread_id.starts_with(&marker) {
            seen.push(ev);
        }
    }
    let scoped = seen.iter().find(|e| e.thread_id == marker).unwrap();
    assert!(scoped.belongs_to("u-alice"));
    assert!(!scoped.belongs_to("u-bob"));
    assert!(scoped.belongs_to_profile("p-alice"));
    assert!(!scoped.belongs_to_profile("p-bob"));
    let unscoped = seen.iter().find(|e| e.thread_id != marker).unwrap();
    assert!(
        !unscoped.belongs_to("u-alice") && !unscoped.belongs_to_profile("p-alice"),
        "unstamped events belong to no user"
    );
    assert!(
        !serde_json::to_string(scoped).unwrap().contains("u-alice"),
        "the stamp is not serialized"
    );
}

/// An async sub-agent's card is marked `detached` on the wire so the client
/// keeps it across the parent turn's `chat_done`; an in-turn card carries no
/// flag at all.
#[test]
fn approval_request_event_marks_only_detached_parks() {
    let build = |detached| {
        serde_json::to_value(approval_request_event(
            "req-1",
            "media_generate_image",
            "a red fox",
            &serde_json::json!({}),
            "thread-1",
            "client-1",
            None,
            None,
            detached,
        ))
        .unwrap()
    };
    assert_eq!(build(true)["detached"], serde_json::json!(true));
    assert!(build(false).get("detached").is_none());
}
