use super::tests::find_agent_web_event;
use super::*;
use crate::web_chat::{QueueItemPayload, ScopedWebEvent, WebChannelEvent, WebEventContext};
use tokio::sync::broadcast;

fn wire(event: &ScopedWebEvent) -> serde_json::Value {
    serde_json::to_value(event).expect("web event serializes")
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
    assert_eq!(ev.context().client_id, "");
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
    assert_eq!(wire(&ev)["todos"], todos);
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
    let item: QueueItemPayload =
        serde_json::from_value(wire(&ev)["queue_item"].clone()).expect("queue_item");
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
    assert_eq!(ev.context().thread_id, thread_id);
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
    let item: QueueItemPayload =
        serde_json::from_value(wire(&ev)["queue_item"].clone()).expect("queue_item");
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
    assert_eq!(ev.context().client_id, "");
    assert_eq!(wire(&ev)["message"], "plan");
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
    assert_eq!(ev.context().client_id, "client-decided-1");
    assert_eq!(ev.context().request_id, "req-decided-1");
    assert_eq!(wire(&ev)["tool_call_id"], "call-decided-1");
    assert_eq!(wire(&ev)["cancel_reason"], "expired");
    assert_eq!(wire(&ev)["message"], "deny");
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
                    if ev.event_name() == "approval_decided"
                        && ev.context().request_id == "req-decided-no-route" =>
                {
                    return Some(ev);
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
    assert_eq!(ev.context().client_id, "client-plan-decided-1");
    assert_eq!(wire(&ev)["tool_call_id"], "call-plan-decided-1");
    assert!(wire(&ev).get("cancel_reason").is_none());
    assert_eq!(wire(&ev)["message"], "approve");
}

/// `publish_web_channel_event` stamps `ts` (epoch ms) when the caller left it
/// unset, so every emitted event carries a wall-clock time even when the
/// producer never set one explicitly.
#[tokio::test]
async fn publish_web_channel_event_stamps_ts_when_unset() {
    let mut web_rx = subscribe_web_channel_events();
    let before = crate::web_chat::progress::unix_epoch_ms();

    publish_web_channel_event(WebChannelEvent::UserError {
        context: WebEventContext {
            thread_id: "thread-ts-stamp-probe".into(),
            ..WebEventContext::default()
        },
        error_type: Some("ts_stamp_probe".into()),
        error_source: None,
    });

    let ev = find_agent_web_event(&mut web_rx, "user_error", "thread-ts-stamp-probe").await;
    let after = crate::web_chat::progress::unix_epoch_ms();
    let ts = ev.context().ts.expect("publisher stamps timestamp");
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

    publish_web_channel_event(WebChannelEvent::ChatDone {
        context: WebEventContext {
            client_id: "ts-probe".into(),
            thread_id: "thread-ts-stamp-probe-explicit".into(),
            request_id: "ts-probe".into(),
            turn_request_id: None,
            seq: None,
            ts: Some(123),
        },
        full_response: None,
        citations: None,
        usage: None,
        timing: None,
        parts: None,
    });

    let ev = find_agent_web_event(&mut web_rx, "chat_done", "thread-ts-stamp-probe-explicit").await;
    assert_eq!(ev.context().ts, Some(123));
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
        publish_web_channel_event(WebChannelEvent::ChatDone {
            context: WebEventContext {
                client_id: String::new(),
                thread_id: marker.clone(),
                request_id: "agent-stamp".into(),
                ..WebEventContext::default()
            },
            full_response: None,
            citations: None,
            usage: None,
            timing: None,
            parts: None,
        });
    })
    .await;
    publish_web_channel_event(WebChannelEvent::ChatDone {
        context: WebEventContext {
            client_id: String::new(),
            thread_id: format!("{marker}-unscoped"),
            request_id: "agent-stamp-unscoped".into(),
            ..WebEventContext::default()
        },
        full_response: None,
        citations: None,
        usage: None,
        timing: None,
        parts: None,
    });

    let mut seen = Vec::new();
    while seen.len() < 2 {
        let ev = rx.recv().await.unwrap();
        if ev.context().thread_id.starts_with(&marker) {
            seen.push(ev);
        }
    }
    let scoped = seen
        .iter()
        .find(|e| e.context().thread_id == marker)
        .unwrap();
    assert!(scoped.belongs_to("u-alice"));
    assert!(!scoped.belongs_to("u-bob"));
    assert!(scoped.belongs_to_profile("p-alice"));
    assert!(!scoped.belongs_to_profile("p-bob"));
    let unscoped = seen
        .iter()
        .find(|e| e.context().thread_id != marker)
        .unwrap();
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
