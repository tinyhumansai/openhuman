use super::*;

#[cfg(feature = "voice")]
#[tokio::test]
async fn ptt_source_turn_completes_without_publishing_a_voice_event() {
    use std::sync::Arc;
    use tinybus::EventHandler;
    use tinyinference_llm::model::{ChatModel, ModelRequest, ModelResponse};

    struct RecordingModel {
        inner: Arc<tinyagents_harness::testkit::ScriptedModel>,
        errors: Arc<std::sync::Mutex<Vec<String>>>,
    }

    #[async_trait::async_trait]
    impl ChatModel<()> for RecordingModel {
        async fn invoke(
            &self,
            state: &(),
            request: ModelRequest,
        ) -> tinyinference_llm::Result<ModelResponse> {
            match self.inner.invoke(state, request).await {
                Ok(response) => Ok(response),
                Err(error) => {
                    self.errors
                        .lock()
                        .expect("recording model error lock")
                        .push(error.to_string());
                    Err(error)
                }
            }
        }
    }

    let _ = env_logger::builder().is_test(true).try_init();
    let _turn_lock = FORCED_ERROR_TEST_LOCK.lock().await;
    let _inference = crate::inference::inference_test_guard_async().await;
    let model = Arc::new(tinyagents_harness::testkit::ScriptedModel::replies(vec![
        "hello from mock",
    ]));
    let model_errors = Arc::new(std::sync::Mutex::new(Vec::new()));
    let recording_model = Arc::new(RecordingModel {
        inner: Arc::clone(&model),
        errors: Arc::clone(&model_errors),
    });
    let _model = crate::inference::provider::factory::test_provider_override::install_model(
        Arc::clone(&recording_model) as Arc<dyn ChatModel<()>>,
    );
    let temp = tempfile::tempdir().expect("temporary test workspace");
    let workspace_dir = temp.path().join("workspace");
    let action_dir = temp.path().join("projects");
    let config_path = temp.path().join("config/config.toml");
    let files_dir = temp.path().join("files");
    std::fs::create_dir_all(&workspace_dir).expect("create isolated workspace");
    std::fs::create_dir_all(&action_dir).expect("create isolated action directory");
    std::fs::create_dir_all(&files_dir).expect("create isolated files directory");
    std::fs::create_dir_all(config_path.parent().unwrap()).expect("create isolated config dir");
    let mut config = crate::config::Config::default();
    config.workspace_dir = workspace_dir;
    config.action_dir = action_dir.clone();
    config.action_dir_override = Some(action_dir.clone());
    config.config_path = config_path;
    config.files_dir_override = Some(files_dir);
    config.local_ai.runtime_enabled = false;
    config.memory.conversations.enabled = false;
    config.agent.session_dual_write = false;
    config.agent.session_shadow_reads = false;
    config.components.mcp = false;
    config.components.memory = false;
    config.components.skills = false;
    config.components.flows = false;
    config.agent.chat_agent_id = Some("ptt_event_regression".to_string());
    let mut definition = crate::agent::harness::test_support::test_main_def();
    definition.id = "ptt_event_regression".to_string();
    definition.tools = tinyagents_definition::ToolScope::Named(Vec::new());
    definition.extra_tools.clear();
    definition.deferred_tools.clear();
    definition.subagents.clear();
    definition.max_iterations = 3;
    definition.sandbox_mode = tinyagents_definition::SandboxMode::None;
    definition.extensions.omit_memory_context = true;
    definition.extensions.searches_connected_mcp = false;
    let definitions = Arc::new(
        crate::agent_registry::host_definition::AgentDefinitionRegistry::builtins_only()
            .with_definitions([definition]),
    );
    let mut components = crate::core::runtime::ComponentSet::all();
    components.mcp = false;
    components.memory = false;
    components.skills = false;
    components.flows = false;
    components.voice = true;
    let mut host_overrides = crate::agent::host_overrides::HostOverrides::default();
    host_overrides.model = Some(recording_model);
    let core_context = crate::core::runtime::CoreContext::for_test_with_components(components)
        .derive_with(
            crate::core::runtime::ContextOverlay::new(
                config,
                crate::core::runtime::DomainSet::full(),
                crate::tools::toolpacks::ToolGroups::none(),
            )
            .without_user_skill_roots()
            .definitions(definitions)
            .host_overrides(Arc::new(host_overrides)),
        );
    let ctx = crate::core::runtime::DispatchCtx::new(Arc::clone(&core_context));

    crate::core::runtime::CoreContext::scope(core_context, async {
        struct Events(tokio::sync::mpsc::UnboundedSender<crate::core::events::DomainEvent>);

        #[async_trait::async_trait]
        impl EventHandler<crate::core::events::DomainEvent> for Events {
            fn name(&self) -> &str {
                "web_chat::ptt_regression_test"
            }

            async fn handle(&self, event: &crate::core::events::DomainEvent) {
                let _ = self.0.send(event.clone());
            }
        }

        crate::core::bus::init().await.expect("event bus init");
        let (events_tx, mut observed_events) = tokio::sync::mpsc::unbounded_channel();
        let _subscription = crate::core::bus::BUS
            .subscribe(Arc::new(Events(events_tx)))
            .expect("voice event subscription");

        let bus_ready = "ptt-regression-bus-ready-403";
        crate::core::bus::BUS.publish(crate::core::events::DomainEvent::SystemStartup {
            component: bus_ready.to_string(),
        });
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if matches!(observed_events.recv().await,
                Some(crate::core::events::DomainEvent::SystemStartup { component })
                    if component == bus_ready)
                {
                    break;
                }
            }
        })
        .await
        .expect("BUS subscriber should receive its control event");

        let mut web_events = subscribe_web_channel_events();
        let thread_id = "ptt-event-regression-403-thread";
        let session_id = 403_042;
        let request_id = start_chat(
            &ctx,
            "ptt-test-client",
            thread_id,
            "recognized words",
            None,
            None,
            None,
            None,
            ChatRequestMetadata {
                source: Some("ptt".to_string()),
                session_id: Some(session_id),
                ..Default::default()
            },
        )
        .await
        .expect("PTT web chat turn should start");

        let done = tokio::time::timeout(std::time::Duration::from_secs(20), async {
            loop {
                let event = web_events
                    .recv()
                    .await
                    .expect("web event stream stays open");
                if event.context().request_id == request_id && event.event_name() == "chat_done" {
                    break event;
                }
                assert_ne!(
                    (event.context().request_id.as_str(), event.event_name()),
                    (request_id.as_str(), "chat_error"),
                    "PTT turn should complete successfully: {event:?}; scripted model received {} requests; invoke errors: {:?}",
                    model.requests().len(),
                    model_errors.lock().expect("recording model error lock").as_slice()
                );
            }
        })
        .await
        .unwrap_or_else(|_| {
            panic!(
                "PTT turn should finish; scripted model received {} requests; invoke errors: {:?}",
                model.requests().len(),
                model_errors.lock().expect("recording model error lock").as_slice()
            )
        });
        let full_response = match done.wire_event() {
            crate::web_chat::WebChannelEvent::ChatDone { full_response, .. } => full_response.clone(),
            _ => None,
        };
        assert_eq!(full_response.as_deref(), Some("hello from mock"));
        assert!(
            !model.requests().is_empty(),
            "successful turn should invoke the scripted model"
        );

        // Subscriber dispatch is a single FIFO loop (`events/subscriber.rs`), so
        // observing this marker proves it has handled every locally published
        // event before the barrier, including any PTT event sent by the turn.
        let barrier = "ptt-regression-bus-barrier-403";
        crate::core::bus::BUS.publish(crate::core::events::DomainEvent::SystemStartup {
            component: barrier.to_string(),
        });
        let matching_voice_event_seen =
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                let mut matching_voice_event_seen = false;
                loop {
                    let Some(event) = observed_events.recv().await else {
                        continue;
                    };
                    if matches!(
                        &event,
                        crate::core::events::DomainEvent::SystemStartup { component }
                            if component == barrier
                    ) {
                        break matching_voice_event_seen;
                    }
                    let debug = format!("{event:?}");
                    if event.domain() == "voice"
                        && debug.contains(thread_id)
                        && debug.contains(&session_id.to_string())
                    {
                        matching_voice_event_seen = true;
                    }
                }
            })
            .await
            .expect("BUS subscriber should reach the post-turn barrier");
        assert!(
            !matching_voice_event_seen,
            "a successful source=ptt message must not publish a voice event"
        );
    })
    .await;
}
