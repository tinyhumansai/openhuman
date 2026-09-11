use super::*;
use tinybus::NativeRegistry;

/// Build a canonical test request. The bus handler is always stubbed
/// in these tests, so the provider trait object is never actually
/// invoked — an empty native scripted model only satisfies the type.
fn test_request() -> AgentTurnRequest {
    let model: Arc<dyn tinyinference::model::ChatModel<()>> =
        Arc::new(tinyagents_harness::testkit::ScriptedModel::new(Vec::new()));
    AgentTurnRequest {
        turn_model_source: crate::openhuman::agent::tinyagents::TurnModelSource::from_model(model),
        history: vec![
            ChatMessage::system("you are a test bot"),
            ChatMessage::user("hello"),
        ],
        tools_registry: Arc::new(Vec::new()),
        provider_name: "fake-provider".into(),
        model: "fake-model".into(),
        temperature: 0.0,
        silent: true,
        channel_name: "test-channel".into(),
        multimodal: MultimodalConfig::default(),
        multimodal_files: crate::openhuman::config::MultimodalFileConfig::default(),
        max_tool_iterations: 1,
        on_delta: None,
        target_agent_id: None,
        visible_tool_names: None,
        extra_tools: Vec::new(),
        on_progress: None,
        origin: AgentTurnOrigin::Cli,
    }
}

#[tokio::test]
async fn registry_override_routes_request_through_bus() {
    // Isolated local registry so this test doesn't fight the global one.
    let registry = NativeRegistry::new();
    registry.register::<AgentTurnRequest, AgentTurnResponse, _, _>(
        AGENT_RUN_TURN_METHOD,
        |req| async move {
            // Prove owned fields arrived intact across the bus boundary.
            assert_eq!(req.provider_name, "fake-provider");
            assert_eq!(req.channel_name, "test-channel");
            assert_eq!(req.history.len(), 2);
            Ok(AgentTurnResponse::new(format!(
                "handled({})",
                req.history.len()
            )))
        },
    );

    let resp = registry
        .request::<AgentTurnRequest, AgentTurnResponse>(AGENT_RUN_TURN_METHOD, test_request())
        .await
        .expect("dispatch should succeed");

    assert_eq!(resp.text, "handled(2)");
}

#[tokio::test]
async fn streaming_delta_channel_survives_bus_roundtrip() {
    // Prove that `mpsc::Sender<String>` — a non-serializable type —
    // passes through the bus unchanged and the handler can write
    // through it. This is the whole reason native_request exists.
    let registry = NativeRegistry::new();
    registry.register::<AgentTurnRequest, AgentTurnResponse, _, _>(
        AGENT_RUN_TURN_METHOD,
        |req| async move {
            let tx = req
                .on_delta
                .expect("streaming test must supply an on_delta sender");
            tx.send("chunk1".into()).await.map_err(|e| e.to_string())?;
            tx.send("chunk2".into()).await.map_err(|e| e.to_string())?;
            Ok(AgentTurnResponse::new("streamed"))
        },
    );

    let (tx, mut rx) = mpsc::channel::<String>(4);
    let collector = tokio::spawn(async move {
        let mut buf = Vec::new();
        while let Some(d) = rx.recv().await {
            buf.push(d);
        }
        buf
    });

    let mut req = test_request();
    req.on_delta = Some(tx);

    let resp = registry
        .request::<AgentTurnRequest, AgentTurnResponse>(AGENT_RUN_TURN_METHOD, req)
        .await
        .expect("dispatch should succeed");

    assert_eq!(resp.text, "streamed");

    let chunks = collector.await.unwrap();
    assert_eq!(chunks, vec!["chunk1".to_string(), "chunk2".to_string()]);
}

#[tokio::test]
async fn register_agent_handlers_exposes_run_turn_on_global_registry() {
    // Read-only smoke test: prove the production registration path
    // actually puts `agent.run_turn` on the global registry. Does
    // NOT dispatch — dispatching from this test would race with any
    // other test that installs a handler override (e.g. the channel
    // dispatch integration tests in `runtime_dispatch.rs`).
    register_agent_handlers();
    let registry = Some(crate::core::bus::BUS.native())
        .expect("native registry should be initialized after register_agent_handlers");
    assert!(
        registry.is_registered(AGENT_RUN_TURN_METHOD),
        "`{AGENT_RUN_TURN_METHOD}` should be registered on the global native registry"
    );
}
