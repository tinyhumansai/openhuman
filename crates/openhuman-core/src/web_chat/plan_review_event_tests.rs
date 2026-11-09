use super::*;
use tokio::sync::broadcast;

#[tokio::test]
async fn parked_plan_review_is_projected_as_the_socket_request_event() {
    crate::core::bus::init().await.expect("bus init");
    let _subscription = crate::core::bus::BUS
        .subscribe(Arc::new(ApprovalSurfaceSubscriber))
        .expect("subscribe to plan review events");
    let mut web_rx = subscribe_web_channel_events();

    crate::core::bus::BUS.publish(DomainEvent::PlanReviewRequested {
        request_id: "plan-socket-projection".into(),
        thread_id: Some("thread-socket-projection".into()),
        client_id: Some("client-socket-projection".into()),
        summary: "Review this plan".into(),
        steps: vec!["Inspect".into(), "Apply".into()],
        tool_call_id: Some("call-socket-projection".into()),
        expires_at: Some("2030-01-01T00:00:00Z".into()),
    });

    let event = tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            match web_rx.recv().await {
                Ok(event)
                    if event.event_name() == "plan_review_request"
                        && event.wire_event().context().request_id == "plan-socket-projection" =>
                {
                    return event;
                }
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => {
                    panic!("web-channel event stream closed")
                }
            }
        }
    })
    .await
    .expect("plan_review_request event arrives");

    let WebChannelEvent::PlanReviewRequest {
        context,
        tool_name,
        message,
        args,
        tool_call_id,
        expires_at,
    } = event.wire_event().clone()
    else {
        panic!("expected a plan review request event");
    };
    assert_eq!(context.thread_id, "thread-socket-projection");
    assert_eq!(context.client_id, "client-socket-projection");
    assert_eq!(tool_name.as_deref(), Some("request_plan_review"));
    assert_eq!(tool_call_id.as_deref(), Some("call-socket-projection"));
    assert_eq!(message.as_deref(), Some("Review this plan"));
    assert_eq!(expires_at.as_deref(), Some("2030-01-01T00:00:00Z"));
    assert_eq!(
        args.as_ref().and_then(|args| args.get("steps")),
        Some(&serde_json::json!(["Inspect", "Apply"]))
    );
}
