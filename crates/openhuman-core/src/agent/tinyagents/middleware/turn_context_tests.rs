use super::*;
use tinyagents_harness::context::RunConfig;

#[tokio::test]
async fn attachment_scope_is_request_local_and_contains_only_typed_authority() {
    let root = std::path::PathBuf::from("/workspace/project");
    let mut data = crate::agent::tinyagents::host::OpenHumanRunContext::new();
    data.origin = Some(
        crate::core::runtime::turn_origin::AgentTurnOrigin::ExternalChannel {
            sender_name: None,
            channel: "test".into(),
            sender: None,
            reply_target: "room".into(),
            message_id: "message".into(),
            history_key: None,
            channel_session: None,
        },
    );
    data.workspace = Some(tinytools::WorkspaceDescriptor::new(root.clone()));
    let mut context = RunContext::new(RunConfig::new("attachment-scope"), data);
    let mut request = ModelRequest::new(Vec::new());
    request.metadata["preserved"] = serde_json::json!(true);

    AttachmentRequestScopeMiddleware
        .before_model(&mut context, &(), &mut request)
        .await
        .expect("scope carrier is installed");

    let mut provider_metadata = request.metadata.clone();
    let scope = crate::attachments::take_request_scope(&mut provider_metadata);
    assert!(scope.external_channel);
    assert_eq!(scope.workspace.as_deref(), Some(root.as_path()));
    assert_eq!(provider_metadata, serde_json::json!({"preserved": true}));
    assert!(request.messages.is_empty());
}

#[tokio::test]
async fn attachment_scope_observes_even_after_an_earlier_middleware_stops() {
    assert!(AttachmentRequestScopeMiddleware.is_observer());
}

#[tokio::test]
async fn transcript_snapshot_keeps_a_completed_failed_tool_row() {
    let sink = Arc::new(std::sync::Mutex::new(TranscriptSnapshot::default()));
    let owner_sink = tinyagents_harness::middleware::library::new_transcript_snapshot_sink();
    let middleware = TranscriptSnapshotMiddleware {
        sink: sink.clone(),
        owner: tinyagents_harness::middleware::library::TranscriptSnapshotMiddleware::new(
            owner_sink.clone(),
        ),
        owner_sink,
    };
    let mut context = RunContext::new(
        RunConfig::new("snapshot-tool-outcome"),
        crate::agent::tinyagents::host::OpenHumanRunContext::new(),
    );
    let mut call = tinyinference_llm::tool::ToolCall {
        id: "failed-call".into(),
        name: "write_file".into(),
        arguments: serde_json::json!({"path": "blocked.txt"}),
        invalid: None,
    };
    middleware
        .before_tool(&mut context, &(), &mut call)
        .await
        .expect("snapshot accepts tool start");
    let invocation =
        tinyagents_harness::middleware::ToolInvocationIdentity::new("failed-call", "write_file");
    let mut result = TaToolResult::error("permission denied");
    middleware
        .after_tool(&mut context, &(), &invocation, &mut result)
        .await
        .expect("snapshot accepts tool completion");

    let snapshot = sink.lock().expect("snapshot");
    assert_eq!(
        snapshot.messages,
        vec![Message::tool("failed-call", "permission denied")]
    );
    assert_eq!(snapshot.tool_outcomes.len(), 1);
    let outcome = &snapshot.tool_outcomes[0];
    assert_eq!(outcome.call_id, "failed-call");
    assert_eq!(outcome.name, "write_file");
    assert_eq!(
        outcome.arguments,
        serde_json::json!({"path": "blocked.txt"})
    );
    assert!(!outcome.success);
    assert_eq!(outcome.content, "permission denied");
}

#[tokio::test]
async fn owner_snapshot_spend_is_not_counted_twice_and_cache_replay_stays_free() {
    let sink = Arc::new(std::sync::Mutex::new(TranscriptSnapshot::default()));
    let owner_sink = tinyagents_harness::middleware::library::new_transcript_snapshot_sink();
    let middleware = TranscriptSnapshotMiddleware {
        sink: sink.clone(),
        owner: tinyagents_harness::middleware::library::TranscriptSnapshotMiddleware::new(
            owner_sink.clone(),
        ),
        owner_sink,
    };
    let mut context = RunContext::new(
        RunConfig::new("snapshot-cache-replay"),
        crate::agent::tinyagents::host::OpenHumanRunContext::new(),
    );
    let first = ModelRequest {
        messages: vec![Message::user("question")],
        ..ModelRequest::default()
    };
    let mut first = first;
    middleware
        .before_model_dispatch(&mut context, &(), &mut first)
        .await
        .expect("snapshot first dispatched request");
    let mut answered = tinyinference_llm::model::ModelResponse::assistant("answer one").with_usage(
        tinyinference_llm::usage::Usage {
            input_tokens: 12,
            output_tokens: 4,
            cache_read_tokens: 3,
            ..Default::default()
        },
    );
    middleware
        .after_model(&mut context, &(), &mut answered)
        .await
        .expect("snapshot first answer");
    {
        let snapshot = sink.lock().expect("host snapshot lock");
        assert_eq!(snapshot.input_tokens, 12);
        assert_eq!(snapshot.output_tokens, 4);
        assert_eq!(snapshot.cached_input_tokens, 3);
    }

    let mut second = ModelRequest {
        messages: vec![Message::user("question"), Message::assistant("answer one")],
        ..ModelRequest::default()
    };
    middleware
        .before_model_dispatch(&mut context, &(), &mut second)
        .await
        .expect("snapshot replay request");
    let mut answered_again = tinyinference_llm::model::ModelResponse::assistant("answer two")
        .with_usage(tinyinference_llm::usage::Usage {
            input_tokens: 8,
            output_tokens: 2,
            cache_read_tokens: 1,
            ..Default::default()
        });
    middleware
        .after_model(&mut context, &(), &mut answered_again)
        .await
        .expect("snapshot second non-cache answer");
    {
        let snapshot = sink.lock().expect("host snapshot lock");
        assert_eq!(snapshot.input_tokens, 20);
        assert_eq!(snapshot.output_tokens, 6);
        assert_eq!(snapshot.cached_input_tokens, 4);
    }

    let mut replay_request = ModelRequest {
        messages: vec![
            Message::user("question"),
            Message::assistant("answer one"),
            Message::assistant("answer two"),
        ],
        ..ModelRequest::default()
    };
    middleware
        .before_model_dispatch(&mut context, &(), &mut replay_request)
        .await
        .expect("snapshot cache replay request");
    let mut replay = tinyinference_llm::model::ModelResponse::assistant("answer three").with_usage(
        tinyinference_llm::usage::Usage {
            input_tokens: 7,
            output_tokens: 3,
            cache_read_tokens: 7,
            ..Default::default()
        },
    );
    replay.served_from_cache = true;
    middleware
        .after_model(&mut context, &(), &mut replay)
        .await
        .expect("snapshot replay answer");

    let snapshot = sink.lock().expect("host snapshot lock");
    assert_eq!(snapshot.model_calls, 3);
    assert_eq!(snapshot.input_tokens, 20);
    assert_eq!(snapshot.output_tokens, 6);
    assert_eq!(snapshot.cached_input_tokens, 4);
    assert_eq!(snapshot.last_call_input_tokens, 7);
    assert_eq!(snapshot.last_call_output_tokens, 3);
}

#[tokio::test]
async fn host_route_fallback_survives_owner_snapshot_syncs() {
    let sink = Arc::new(std::sync::Mutex::new(TranscriptSnapshot::default()));
    let owner_sink = tinyagents_harness::middleware::library::new_transcript_snapshot_sink();
    let middleware = TranscriptSnapshotMiddleware {
        sink: sink.clone(),
        owner: tinyagents_harness::middleware::library::TranscriptSnapshotMiddleware::new(
            owner_sink.clone(),
        ),
        owner_sink,
    };
    let mut context = RunContext::new(
        RunConfig::new("snapshot-route-fallback"),
        crate::agent::tinyagents::host::OpenHumanRunContext::new(),
    );
    let mut request = ModelRequest::default();
    middleware
        .before_model_dispatch(&mut context, &(), &mut request)
        .await
        .expect("snapshot dispatched request");
    let mut first = ModelResponse::assistant("first");
    first.resolved_route = Some(ResolvedModelRoute::new("provider", "model-a", "route-a"));
    middleware
        .after_model(&mut context, &(), &mut first)
        .await
        .expect("snapshot first route");

    *context.data.resolved_route.lock().expect("route lock") =
        Some(ResolvedModelRoute::new("provider", "model-b", "route-b"));
    let mut second_request = ModelRequest::default();
    middleware
        .before_model_dispatch(&mut context, &(), &mut second_request)
        .await
        .expect("snapshot next dispatched request");
    let mut second = ModelResponse::assistant("second");
    middleware
        .after_model(&mut context, &(), &mut second)
        .await
        .expect("snapshot host route fallback");
    middleware
        .before_model_dispatch(&mut context, &(), &mut ModelRequest::default())
        .await
        .expect("later sync leaves host route intact");

    assert_eq!(
        sink.lock().expect("host snapshot lock").resolved_route,
        Some(ResolvedModelRoute::new("provider", "model-b", "route-b"))
    );
}
