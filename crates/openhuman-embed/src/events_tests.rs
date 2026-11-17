use super::*;

#[tokio::test]
async fn event_order_is_stable_and_lag_is_visible() {
    let hub = EventHub::new(2);
    let mut events = hub.subscribe();
    for _ in 0..4 {
        hub.emit(
            Some("a".into()),
            Some("t".into()),
            RuntimeEventKind::TurnStarted,
        );
    }
    assert_eq!(events.recv().await, Err(EventStreamError::Lagged(2)));
    assert_eq!(events.recv().await.unwrap().sequence, 3);
    assert_eq!(events.recv().await.unwrap().sequence, 4);
    drop(hub);
    assert_eq!(events.recv().await, Err(EventStreamError::Closed));
}

#[test]
fn observations_serialize_only_metadata() {
    let event = RuntimeEvent {
        sequence: 1,
        agent_id: Some("a".into()),
        turn_id: Some("t".into()),
        kind: RuntimeEventKind::ToolStarted {
            tool_name: "read".into(),
        },
    };
    let value = serde_json::to_value(event).unwrap();
    assert_eq!(value["kind"]["kind"], "tool_started");
    assert!(value.get("arguments").is_none());
    assert!(value.get("message").is_none());
}

fn requested(agent: Option<&str>, thread: Option<&str>, id: &str) -> DomainEvent {
    DomainEvent::ApprovalRequested {
        request_id: id.into(),
        tool_name: "write".into(),
        action_summary: "safe summary".into(),
        args_redacted: serde_json::Value::Null,
        thread_id: thread.map(str::to_owned),
        client_id: None,
        tool_call_id: None,
        expires_at: None,
        agent_id: agent.map(str::to_owned),
    }
}

fn no_event(events: &mut RuntimeEvents) {
    let mut context = Context::from_waker(std::task::Waker::noop());
    assert!(Pin::new(events).poll_next(&mut context).is_pending());
}

#[tokio::test]
async fn global_approvals_are_visible_only_for_this_runtimes_registered_agents() {
    let hub = EventHub::new(16);
    hub.emit(Some("local".into()), None, RuntimeEventKind::AgentAdded);
    let mut events = hub.subscribe();
    let handler = ApprovalEvents(Arc::downgrade(&hub));
    handler
        .handle(&requested(
            Some("foreign"),
            Some("thread"),
            "foreign-request",
        ))
        .await;
    handler
        .handle(&requested(None, Some("thread"), "operator-request"))
        .await;
    no_event(&mut events);
    handler
        .handle(&requested(Some("local"), Some("thread"), "local-request"))
        .await;
    assert!(
        matches!(events.recv().await.unwrap().kind, RuntimeEventKind::ApprovalRequested { request_id } if request_id == "local-request")
    );
}

fn decided(agent: &str, thread: &str, id: &str) -> DomainEvent {
    DomainEvent::ApprovalDecided {
        request_id: id.into(),
        tool_name: "write".into(),
        decision: "approve_once".into(),
        thread_id: Some(thread.into()),
        client_id: None,
        tool_call_id: None,
        resolution: None,
        agent_id: Some(agent.into()),
    }
}

#[tokio::test]
async fn repeated_conversation_calls_have_unique_turn_ids_and_approvals_keep_origin() {
    let hub = EventHub::new(16);
    hub.emit(Some("a".into()), None, RuntimeEventKind::AgentAdded);
    let mut events = hub.subscribe();
    let handler = ApprovalEvents(Arc::downgrade(&hub));
    let first = hub.begin_turn(Some("a".into()), "thread");
    assert_ne!(first, "thread");
    assert_eq!(
        events.recv().await.unwrap().turn_id.as_deref(),
        Some(first.as_str())
    );
    handler
        .handle(&requested(Some("a"), Some("thread"), "approval-1"))
        .await;
    assert_eq!(
        events.recv().await.unwrap().turn_id.as_deref(),
        Some(first.as_str())
    );
    hub.end_turn(Some("a".into()), "thread", &first, true);
    assert_eq!(
        events.recv().await.unwrap().turn_id.as_deref(),
        Some(first.as_str())
    );
    let second = hub.begin_turn(Some("a".into()), "thread");
    assert_ne!(first, second);
    assert_eq!(
        events.recv().await.unwrap().turn_id.as_deref(),
        Some(second.as_str())
    );
    // A delayed decision for the first call must not be attributed to the second.
    handler.handle(&decided("a", "thread", "approval-1")).await;
    assert_eq!(
        events.recv().await.unwrap().turn_id.as_deref(),
        Some(first.as_str())
    );
    handler
        .handle(&requested(Some("a"), Some("thread"), "approval-2"))
        .await;
    assert_eq!(
        events.recv().await.unwrap().turn_id.as_deref(),
        Some(second.as_str())
    );
}

#[tokio::test]
async fn concurrent_calls_in_one_thread_are_not_misattributed_and_removed_agents_are_filtered() {
    let hub = EventHub::new(16);
    hub.emit(Some("a".into()), None, RuntimeEventKind::AgentAdded);
    let first = hub.begin_turn(Some("a".into()), "thread");
    let second = hub.begin_turn(Some("a".into()), "thread");
    let mut events = hub.subscribe();
    let handler = ApprovalEvents(Arc::downgrade(&hub));
    handler
        .handle(&requested(Some("a"), Some("thread"), "ambiguous"))
        .await;
    assert_eq!(events.recv().await.unwrap().turn_id, None);
    hub.end_turn(Some("a".into()), "thread", &second, false);
    events.recv().await.unwrap();
    handler
        .handle(&requested(Some("a"), Some("thread"), "unique"))
        .await;
    assert_eq!(
        events.recv().await.unwrap().turn_id.as_deref(),
        Some(first.as_str())
    );
    hub.emit(Some("a".into()), None, RuntimeEventKind::AgentRemoved);
    events.recv().await.unwrap();
    handler
        .handle(&requested(Some("a"), Some("thread"), "removed"))
        .await;
    handler.handle(&decided("a", "thread", "unique")).await;
    no_event(&mut events);
    assert!(hub.state.lock().unwrap().active.is_empty());
    assert!(hub.state.lock().unwrap().approvals.is_empty());
}
