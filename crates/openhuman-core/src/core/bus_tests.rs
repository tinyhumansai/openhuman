use super::*;

#[test]
fn the_catalog_address_constants_are_valid() {
    // These are `expect`ed at runtime, so a typo must fail here rather than
    // at startup in a user's process.
    let _ = config();
    let _ = manifest();
}

#[test]
fn the_manifest_declares_the_catalog_in_both_directions() {
    let manifest = manifest();
    let interface = EVENTS_INTERFACE.try_into().unwrap();
    assert!(
        manifest.provided(&interface).is_some(),
        "openhuman publishes"
    );
    assert!(
        manifest.consumed(&interface).is_some(),
        "openhuman subscribes"
    );
}

#[test]
fn removing_cron_delivery_moves_past_the_canonical_v2_event_catalog() {
    // The canonical 2.0 catalog already retired the desktop voice event, and
    // 2.1 added inbound-event queues. Removing CronDeliveryRequested from
    // that wire contract requires a new major version beyond both.
    let manifest = manifest();
    let interface = EVENTS_INTERFACE.try_into().unwrap();
    let advertised = manifest
        .provided(&interface)
        .expect("OpenHuman advertises its event catalog");
    assert!(
        advertised.speaks.major > 2,
        "removing CronDeliveryRequested is incompatible with the canonical 2.x catalog; got {}",
        advertised.speaks
    );
}

#[test]
fn removed_cron_delivery_is_not_a_deserializable_wire_event() {
    let error = serde_json::from_value::<crate::core::events::DomainEvent>(serde_json::json!({
        "CronDeliveryRequested": {}
    }))
    .expect_err("the removed event must not remain in the published vocabulary");
    let message = error.to_string();
    assert!(
        message.contains("unknown variant") && message.contains("CronDeliveryRequested"),
        "expected an explicit removed-variant error, got {message}"
    );
}

#[test]
fn cron_completion_tenant_remains_optional_and_round_trips() {
    let event = crate::core::events::DomainEvent::CronJobCompleted {
        job_id: "job-1".into(),
        success: true,
        output: "done".into(),
        tenant: Some(crate::core::runtime::Tenant {
            profile: Some("profile-a".into()),
            agent: Some("agent-a".into()),
        }),
    };
    let wire = serde_json::to_value(event).unwrap();
    assert_eq!(wire["CronJobCompleted"]["tenant"]["profile"], "profile-a");
    assert_eq!(wire["CronJobCompleted"]["tenant"]["agent"], "agent-a");

    let legacy = serde_json::from_value::<crate::core::events::DomainEvent>(serde_json::json!({
        "CronJobCompleted": {
            "job_id": "job-legacy",
            "success": true,
            "output": "done"
        }
    }))
    .unwrap();
    assert!(matches!(
        legacy,
        crate::core::events::DomainEvent::CronJobCompleted { tenant: None, .. }
    ));
}

#[tokio::test]
async fn isolated_bus_delivers() {
    use std::sync::Arc;
    use tinybus::EventHandler;
    use tokio::sync::Mutex;

    struct Capture(Arc<Mutex<Vec<DomainEvent>>>);

    #[async_trait::async_trait]
    impl EventHandler<DomainEvent> for Capture {
        fn name(&self) -> &str {
            "test::capture"
        }
        async fn handle(&self, event: &DomainEvent) {
            self.0.lock().await.push(event.clone());
        }
    }

    let bus = crate::core::bus_testing::isolated_bus().await;

    let seen = Arc::new(Mutex::new(Vec::new()));
    let _handle = bus.subscribe(Arc::new(Capture(seen.clone())));
    bus.publish(DomainEvent::SystemStartup {
        component: "test".into(),
    });

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    while seen.lock().await.is_empty() {
        assert!(tokio::time::Instant::now() < deadline, "no delivery");
        tokio::task::yield_now().await;
    }
}

/// The bus logger runs at `debug`, so it is on whenever a developer turns
/// logging up. It must name each event without printing its payload: a channel
/// message or a dictated voice transcript is user content.
#[tokio::test]
async fn the_bus_logger_names_events_without_their_user_content() {
    use tinybus::EventHandler;

    #[derive(Clone, Default)]
    struct Logs(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for Logs {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    const USER_TEXT: &str = "the dictated voice request";
    let events = [
        DomainEvent::ConversationTurnCommitted {
            thread_id: "thread".into(),
            agent_id: Some("orchestrator".into()),
            workspace: None,
            channel: Some("telegram".into()),
            user_text: USER_TEXT.into(),
            assistant_text: "reply".into(),
            tool_calls: Vec::new(),
            workspace_dir: std::path::PathBuf::from("/workspace"),
        },
        DomainEvent::ChannelMessageProcessed {
            channel: "telegram".into(),
            message_id: "telegram_42_20".into(),
            sender: "42".into(),
            reply_target: "42".into(),
            content: USER_TEXT.into(),
            thread_ts: None,
            response: "reply".into(),
            provider: "agent:orchestrator".into(),
            model: "gpt-4o".into(),
            elapsed_ms: 1,
            success: true,
            workspace_dir: std::path::PathBuf::from("/workspace"),
        },
    ];

    let logs = Logs::default();
    let writer = logs.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    tracing::callsite::rebuild_interest_cache();

    for event in &events {
        TracingSubscriber.handle(event).await;
    }

    let text = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
    for name in ["ConversationTurnCommitted", "ChannelMessageProcessed"] {
        assert!(
            text.contains(name),
            "the bus logger must still name {name}: {text}"
        );
    }
    assert!(
        !text.contains(USER_TEXT),
        "user content reached the log: {text}"
    );
    assert!(
        !text.contains("reply"),
        "the reply is user-facing content too: {text}"
    );
}
