use super::*;

#[tokio::test]
async fn disabled_export_does_not_create_a_trace_context() {
    // Usage sharing is on by default, so "disabled" has to turn both exporters
    // off explicitly rather than lean on the default config.
    let mut config = crate::config::Config::default();
    config.observability.share_usage_data = false;
    config.observability.agent_tracing.enabled = false;
    let context = trace_context(
        &config,
        &ChatRequestMetadata::default(),
        "socket-client",
        "thread-123",
        "request-456",
    )
    .await;
    assert!(context.is_none());
}

#[tokio::test]
async fn trace_context_keeps_request_session_group_and_transport_client_distinct() {
    let mut config = crate::config::Config::default();
    config.observability.share_usage_data = false;
    config.observability.agent_tracing.enabled = true;
    config.observability.agent_tracing.capture_content = false;
    let metadata = ChatRequestMetadata {
        source: Some("autonomous".into()),
        session_id: Some(93),
        agent_id: Some("writer".into()),
        ..Default::default()
    };

    let context = trace_context(
        &config,
        &metadata,
        "socket-client-7",
        "conversation-4",
        "request-2",
    )
    .await
    .expect("local tracing enabled");

    assert_eq!(context.session_id, "93:request-2");
    assert_eq!(context.session_group.as_deref(), Some("conversation-4"));
    assert_eq!(context.client_id.as_deref(), Some("socket-client-7"));
    assert_eq!(context.agent_id.as_deref(), Some("writer"));
    assert_eq!(context.channel_source.as_deref(), Some("autonomous"));
    assert_eq!(context.run_type.as_str(), "autonomous_task");
    assert!(!context.capture_content);

    let run_id = tinyagents_harness::ids::RunId::new("host-built-run");
    let observations = [
        tinyagents_harness::observability::AgentObservation {
            event_id: tinyagents_harness::ids::EventId::new("host-built-run-evt-0"),
            run_id: run_id.clone(),
            parent_run_id: None,
            root_run_id: run_id.clone(),
            offset: 0,
            ts_ms: 1_000,
            event: tinyagents_harness::events::AgentEvent::RunStarted {
                run_id: run_id.clone(),
                thread_id: None,
            },
        },
        tinyagents_harness::observability::AgentObservation {
            event_id: tinyagents_harness::ids::EventId::new("host-built-run-evt-1"),
            run_id: run_id.clone(),
            parent_run_id: None,
            root_run_id: run_id.clone(),
            offset: 1,
            ts_ms: 1_010,
            event: tinyagents_harness::events::AgentEvent::RunCompleted {
                run_id: run_id.clone(),
                outcome: None,
            },
        },
    ];
    let mut collector =
        tinyagents_harness::observability::trace_export::collector::SpanCollector::new(
            context.with_run_lineage(
                Some(run_id.as_str().into()),
                None,
                Some(run_id.as_str().into()),
            ),
        );
    for observation in &observations {
        collector.record(observation);
    }
    collector.finish(1_010);
    let root = collector
        .spans()
        .iter()
        .find(|span| span.kind == tinyagents_harness::observability::trace_export::SpanKind::Turn)
        .expect("the host trace context reaches native root projection");
    for (key, expected) in [
        ("client.id", "socket-client-7"),
        ("agent.id", "writer"),
        ("channel.source", "autonomous"),
        ("run.type", "autonomous_task"),
        ("run.id", "host-built-run"),
        ("root_run_id", "host-built-run"),
    ] {
        assert_eq!(root.attributes[key], expected);
    }
    let serialized = tinyagents_harness::observability::trace_export::serialize::spans_to_ndjson(
        tinyagents_harness::observability::trace_export::serialize::SpanEnvelope::Otel,
        collector.spans(),
    );
    let serialized_root: serde_json::Value =
        serde_json::from_str(serialized.lines().next().unwrap())
            .expect("host-built context serializes as valid trace output");
    assert_eq!(
        serialized_root["attributes"]["client.id"],
        "socket-client-7"
    );
    assert_eq!(serialized_root["attributes"]["agent.id"], "writer");
    assert_eq!(serialized_root["attributes"]["run.type"], "autonomous_task");
}
