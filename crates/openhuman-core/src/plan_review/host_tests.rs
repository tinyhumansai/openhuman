use super::*;

struct EventCollector(tokio::sync::mpsc::UnboundedSender<DomainEvent>);

#[async_trait::async_trait]
impl tinybus::EventHandler<DomainEvent> for EventCollector {
    fn name(&self) -> &str {
        "plan-review-host-test"
    }

    async fn handle(&self, event: &DomainEvent) {
        let _ = self.0.send(event.clone());
    }
}

async fn next_for_request(
    events: &mut tokio::sync::mpsc::UnboundedReceiver<DomainEvent>,
    request_id: &str,
    requested: bool,
) -> DomainEvent {
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            let event = events.recv().await.expect("event stream remains open");
            let matches = match &event {
                DomainEvent::PlanReviewRequested { request_id: id, .. } => {
                    requested && id == request_id
                }
                DomainEvent::PlanReviewDecided { request_id: id, .. } => {
                    !requested && id == request_id
                }
                _ => false,
            };
            if matches {
                return event;
            }
        }
    })
    .await
    .expect("plan-review event arrives")
}

#[tokio::test]
async fn request_event_replay_and_rpc_decision_share_the_harness_store() {
    crate::core::bus::init().await.expect("bus init");
    let (tx, mut events) = tokio::sync::mpsc::unbounded_channel();
    let _subscription = BUS
        .subscribe(Arc::new(EventCollector(tx)))
        .expect("subscribe to core events");

    let dispatch = DispatchCtx::ambient();
    let tenant = dispatch.tenant().expect("ambient tenant").profile_only();
    let thread_id = format!("plan-review-host-{}", uuid::Uuid::new_v4());
    let scope = socket_scope_for_dispatch(&dispatch).expect("trusted socket scope");
    let store = global();
    let request_thread = thread_id.clone();
    let parked = tokio::spawn(async move {
        store
            .request_review(
                tenant,
                request_thread,
                Some("test-client".into()),
                PlanReviewRequest {
                    summary: "Review the change".into(),
                    steps: vec!["Inspect".into(), "Apply".into()],
                    tool_call_id: Some("call-123".into()),
                },
            )
            .await
    });

    let replay = tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            if let Some(review) = parked_review_for_socket(&scope, &thread_id) {
                break review;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("pending review is available for replay");
    let requested = next_for_request(&mut events, &replay.request_id, true).await;
    let DomainEvent::PlanReviewRequested { request_id, .. } = requested else {
        unreachable!("filtered to PlanReviewRequested")
    };
    assert_eq!(replay.request_id, request_id);
    assert_eq!(replay.thread_id, thread_id);
    assert_eq!(replay.summary, "Review the change");
    assert_eq!(replay.steps, vec!["Inspect".to_owned(), "Apply".to_owned()]);
    assert_eq!(replay.client_id.as_deref(), Some("test-client"));
    assert_eq!(replay.tool_call_id.as_deref(), Some("call-123"));
    assert!(!replay.expires_at.is_empty());

    assert!(
        decide(&dispatch, &request_id, PlanReviewResolution::Approve).expect("RPC tenant resolves")
    );
    assert_eq!(
        parked.await.expect("park task completes"),
        PlanReviewResolution::Approve
    );
    let decided = next_for_request(&mut events, &request_id, false).await;
    assert!(matches!(
        decided,
        DomainEvent::PlanReviewDecided {
            decision,
            thread_id: Some(id),
            client_id: Some(client),
            tool_call_id: Some(call_id),
            resolution: None,
            ..
        } if decision == "approve"
            && id == thread_id
            && client == "test-client"
            && call_id == "call-123"
    ));
}
