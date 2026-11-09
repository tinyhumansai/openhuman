use super::*;
use crate::agent_registry::host_definition::AgentDefinitionRegistry;
use crate::core::events::DomainEvent;
use std::sync::{Arc, Mutex};
use tinyinference_llm::model::{
    ChatModel, ModelProfile, ModelRequest, ModelResponse, ModelStream, ModelStreamItem,
};
use tinyinference_llm::providers::overrides::{ChatModelEntry, ProviderModelMetadata};

const TEST_MANAGED_MODEL: &str = "test/triage-managed";

struct TriageReplyModel {
    requests: Arc<Mutex<Vec<(String, String, ModelRequest)>>>,
    role: String,
    model: String,
}

#[async_trait::async_trait]
impl ChatModel<()> for TriageReplyModel {
    fn profile(&self) -> Option<&ModelProfile> {
        static PROFILE: std::sync::LazyLock<ModelProfile> =
            std::sync::LazyLock::new(|| ModelProfile {
                provider: Some("triage-escalation-test".into()),
                tool_calling: true,
                parallel_tool_calls: true,
                streaming: true,
                ..ModelProfile::default()
            });
        Some(&PROFILE)
    }

    async fn invoke(
        &self,
        _state: &(),
        request: ModelRequest,
    ) -> tinyinference_llm::Result<ModelResponse> {
        self.requests
            .lock()
            .unwrap()
            .push((self.role.clone(), self.model.clone(), request));
        Ok(ModelResponse::assistant("handled inline"))
    }

    async fn stream(
        &self,
        _state: &(),
        request: ModelRequest,
    ) -> tinyinference_llm::Result<ModelStream> {
        self.requests
            .lock()
            .unwrap()
            .push((self.role.clone(), self.model.clone(), request));
        let response = ModelResponse::assistant("handled inline");
        Ok(ModelStream::new(Box::pin(futures::stream::iter([
            ModelStreamItem::Started,
            ModelStreamItem::MessageDelta(tinyinference_llm::message::MessageDelta::text(
                "handled inline",
            )),
            ModelStreamItem::Completed(response),
        ]))))
    }
}

fn triage_test_model(
    requests: Arc<Mutex<Vec<(String, String, ModelRequest)>>>,
    builds: Arc<Mutex<Vec<(String, String)>>>,
    failing_role: Option<&'static str>,
) -> ChatModelEntry<crate::inference::registry::ProviderContext> {
    ChatModelEntry {
        build: Arc::new(move |build_request| {
            let role = build_request.role.to_string();
            let model = build_request
                .model_override
                .or(build_request.default_model)
                .unwrap_or("unknown-test-model")
                .to_string();
            builds.lock().unwrap().push((role.clone(), model.clone()));
            if failing_role == Some(build_request.role) {
                return Err(tinyinference_llm::Error::Model(format!(
                    "test provider has no model for role {role}"
                )));
            }
            Ok((
                Arc::new(TriageReplyModel {
                    requests: Arc::clone(&requests),
                    role,
                    model: model.clone(),
                }) as Arc<dyn ChatModel<()>>,
                model,
            ))
        }),
        probe: None,
        models: Some(triage_model_metadata()),
    }
}

/// Model vocabulary for the test provider. The managed factory resolves the
/// model through the provider's metadata and drops `hint:*` pins from the
/// plain fallback, so this provider turns every hint (and an unpinned role)
/// into the concrete managed model the configuration names.
fn triage_model_metadata() -> ProviderModelMetadata<crate::inference::registry::ProviderContext> {
    let hint_role = |model: &str| {
        model
            .strip_prefix("hint:")
            .filter(|role| crate::config::WORKLOAD_ROLES.contains(role))
            .map(str::to_string)
    };
    ProviderModelMetadata {
        default_model: Arc::new(|| Some(TEST_MANAGED_MODEL.to_string())),
        role_for_model: Arc::new(move |model| hint_role(model)),
        is_alias: Arc::new(|model| model.starts_with("hint:")),
        is_raw_model: Arc::new(|model| !model.starts_with("hint:")),
        supports_vision: Arc::new(|_| false),
        resolve_model: Arc::new(|_role, context, model| match model {
            Some(model) if !model.starts_with("hint:") => Some(model.to_string()),
            _ => Some(
                context
                    .config
                    .default_model
                    .as_deref()
                    .filter(|configured| !configured.starts_with("hint:"))
                    .unwrap_or(TEST_MANAGED_MODEL)
                    .to_string(),
            ),
        }),
        context_window: Arc::new(|_| None),
        default_pricing: Arc::new(|| None),
        without_reasoning: Arc::new(|request| request),
    }
}

#[tokio::test]
async fn react_dispatch_keeps_routing_without_creating_child_lifecycle_records() {
    let _events_guard = test_events_guard().await;
    let temp = tempfile::tempdir().expect("isolated triage workspace");
    let workspace = temp.path().join("workspace");
    let mut config = Config {
        workspace_dir: workspace.clone(),
        config_path: temp.path().join("config.toml"),
        action_dir: temp.path().join("actions"),
        ..Config::default()
    };
    config.default_model = Some(TEST_MANAGED_MODEL.to_string());

    let requests = Arc::new(Mutex::new(Vec::new()));
    let builds = Arc::new(Mutex::new(Vec::new()));
    let extensions = crate::core::slots::Extensions::new();
    crate::inference::registry::register_chat_model_in(
        &extensions,
        crate::inference::provider::factory::PROVIDER_OPENHUMAN,
        triage_test_model(Arc::clone(&requests), Arc::clone(&builds), None),
    );
    let mut domains = crate::core::runtime::DomainSet::harness();
    domains.platform = true;
    let runtime = crate::core::runtime::CoreBuilder::new(crate::core::types::HostKind::Library)
        .services(crate::core::runtime::ServiceSet::none())
        .domains(domains)
        .tool_groups(crate::tools::toolpacks::ToolGroups::advertised())
        .config(config)
        .extensions(extensions)
        .build()
        .await
        .expect("isolated core runtime");
    let ctx = crate::core::runtime::DispatchCtx::new(Arc::clone(runtime.context()));
    let envelope = envelope("esc-react-inline");
    crate::core::bus::init().await.expect("bus init");

    runtime
        .run_in(
            Arc::clone(runtime.context()),
            crate::core::runtime::turn_origin::with_origin(
                crate::triage::local_trigger_origin(),
                apply_decision(
                    &ctx,
                    run_with_target(
                        TriageAction::React,
                        "trigger_reactor",
                        "handle this trigger inline",
                    ),
                    &envelope,
                ),
            ),
        )
        .await
        .expect("react escalation should complete");

    // Prove that the requested route reached the model before checking the
    // lifecycle boundary below, so a skipped dispatch cannot satisfy it.
    {
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 1, "triage should make one model request");
        assert_eq!(
            requests[0].0, "agentic",
            "the trigger reactor should use its configured model role"
        );
        assert_eq!(
            requests[0].1, TEST_MANAGED_MODEL,
            "the configured managed model pin should reach the actual turn model"
        );
        assert!(
            requests[0]
                .2
                .messages
                .iter()
                .any(|message| message.text().contains("handle this trigger inline")),
            "the target agent should receive the triage prompt"
        );
        let tool_names: Vec<_> = requests[0]
            .2
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect();
        assert!(tool_names.contains(&"read_workspace_state"));
        assert!(!tool_names
            .iter()
            .any(|name| { *name == "spawn_subagent" || name.starts_with("delegate_") }));
        assert!(!tool_names.contains(&"shell"));
        assert!(!tool_names.contains(&"file_write"));
    }
    let captured = events::test_events_for_external_id("esc-react-inline");
    assert!(captured.iter().any(|event| matches!(
        event,
        DomainEvent::TriggerEscalated {
            external_id,
            target_agent,
            ..
        } if external_id == "esc-react-inline" && target_agent == "trigger_reactor"
    )));
    assert!(!captured.iter().any(|event| matches!(
        event,
        DomainEvent::TriggerEscalationFailed {
            external_id,
            ..
        } if external_id == "esc-react-inline"
    )));

    let child_lifecycle = runtime
        .run_in(Arc::clone(runtime.context()), async {
            crate::storage::state_kv::StateKv::open(&workspace, "subagents", "lifecycle-neutral")
                .expect("open scoped neutral subagent lifecycle records")
                .list::<serde_json::Value>("record/")
        })
        .await
        .expect("read scoped neutral subagent lifecycle records");
    assert!(
        child_lifecycle.is_empty(),
        "a triage one-shot turn must not persist a child subagent lifecycle: {child_lifecycle:?}"
    );
}

#[tokio::test]
async fn direct_turn_model_source_metadata_probe_does_not_duplicate_egress_disclosure() {
    use crate::core::bus::BUS;
    use crate::security::egress::EgressDescriptor;
    use std::time::Duration;

    let _events_guard = test_events_guard().await;
    crate::core::bus::init().await.expect("bus init");
    let mut rx = BUS.get().expect("event bus").receiver();
    let temp = tempfile::tempdir().expect("isolated triage workspace");
    let workspace = temp.path().join("workspace");
    let mut config = Config {
        workspace_dir: workspace.clone(),
        config_path: temp.path().join("config.toml"),
        action_dir: temp.path().join("actions"),
        ..Config::default()
    };
    let model_marker = "triage-direct-source-egress-marker";
    config.default_model = Some(model_marker.to_string());

    let requests = Arc::new(Mutex::new(Vec::new()));
    let builds = Arc::new(Mutex::new(Vec::new()));
    let extensions = crate::core::slots::Extensions::new();
    crate::inference::registry::register_chat_model_in(
        &extensions,
        crate::inference::provider::factory::PROVIDER_OPENHUMAN,
        triage_test_model(Arc::clone(&requests), Arc::clone(&builds), None),
    );
    let runtime = crate::core::runtime::CoreBuilder::new(crate::core::types::HostKind::Library)
        .services(crate::core::runtime::ServiceSet::none())
        .domains(crate::core::runtime::DomainSet::none())
        .config(config.clone())
        .extensions(extensions)
        .build()
        .await
        .expect("isolated core runtime");
    // The turn runs hosted, so the context must carry the agent catalogue the
    // hosted invocation base is built from.
    let context = runtime.context().derive_with(
        crate::core::runtime::ContextOverlay::new(
            config.clone(),
            crate::core::runtime::DomainSet::none(),
            crate::tools::toolpacks::ToolGroups::none(),
        )
        .definitions(Arc::new(AgentDefinitionRegistry::builtins_only())),
    );
    let ctx = crate::core::runtime::DispatchCtx::new(Arc::clone(&context));
    let definition = AgentDefinitionRegistry::builtins_only()
        .get("trigger_reactor")
        .expect("built-in trigger reactor exists")
        .clone();
    let source = crate::agent::tinyagents::TurnModelSource::new_crate_native(
        &ctx,
        "chat",
        Arc::new(config.clone()),
    );

    runtime
        .run_in(
            context,
            crate::core::runtime::turn_origin::with_origin(
                crate::triage::local_trigger_origin(),
                crate::agent_runs::agent_chat::agent_chat_for_with_turn_model_source(
                    &mut config,
                    crate::inference::host_runtime::ops::AgentChatTarget::Definition {
                        definition: &definition,
                        host: None,
                        seed: None,
                        usage: None,
                        host_only: false,
                        untrusted_input: false,
                        shape: None,
                        observer: None,
                        attachments: &[],
                    },
                    "send this trigger reply",
                    None,
                    Some(definition.temperature),
                    None,
                    None,
                    None,
                    crate::inference::host_runtime::ops::AgentChatEnv::default(),
                    (source, model_marker.to_string()),
                ),
            ),
        )
        .await
        .expect("direct target turn should complete");

    {
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 1, "the model turn ran");
        assert_eq!(
            requests[0].1, model_marker,
            "the expected model served the turn"
        );
        assert!(requests[0]
            .2
            .messages
            .iter()
            .any(|message| { message.text().contains("send this trigger reply") }));
    }
    let sentinel = "triage-direct-source-egress-sentinel";
    BUS.publish(crate::core::events::DomainEvent::ExternalTransferPending {
        descriptor: EgressDescriptor::network_fetch(sentinel),
        thread_id: None,
        client_id: None,
        request_id: None,
    });
    let mut disclosures = 0;
    let mut observed_destinations = Vec::new();
    loop {
        match tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("event bus sentinel should arrive")
        {
            Some(crate::core::events::DomainEvent::ExternalTransferPending {
                descriptor, ..
            }) if descriptor.service == model_marker => {
                disclosures += 1;
                observed_destinations.push((descriptor.provider_slug, descriptor.service));
            }
            Some(crate::core::events::DomainEvent::ExternalTransferPending {
                descriptor, ..
            }) if descriptor.provider_slug == "openhuman"
                && matches!(
                    descriptor.reason,
                    crate::security::egress::EgressReason::Inference
                ) =>
            {
                observed_destinations.push((descriptor.provider_slug, descriptor.service));
            }
            Some(crate::core::events::DomainEvent::ExternalTransferPending {
                descriptor, ..
            }) if descriptor.service == sentinel => break,
            Some(_) => continue,
            None => panic!("event bus closed before egress sentinel"),
        }
    }
    assert_eq!(
        disclosures, 1,
        "one direct inference turn should disclose its model destination once; observed {observed_destinations:?}"
    );
}

#[tokio::test]
async fn react_dispatch_falls_back_to_parent_model_when_hint_route_cannot_build() {
    let _events_guard = test_events_guard().await;
    let temp = tempfile::tempdir().expect("isolated triage workspace");
    let workspace = temp.path().join("workspace");
    let mut config = Config {
        workspace_dir: workspace.clone(),
        config_path: temp.path().join("config.toml"),
        action_dir: temp.path().join("actions"),
        ..Config::default()
    };
    config.default_model = Some(TEST_MANAGED_MODEL.to_string());

    let requests = Arc::new(Mutex::new(Vec::new()));
    let builds = Arc::new(Mutex::new(Vec::new()));
    let extensions = crate::core::slots::Extensions::new();
    crate::inference::registry::register_chat_model_in(
        &extensions,
        crate::inference::provider::factory::PROVIDER_OPENHUMAN,
        triage_test_model(Arc::clone(&requests), Arc::clone(&builds), Some("agentic")),
    );
    let runtime = crate::core::runtime::CoreBuilder::new(crate::core::types::HostKind::Library)
        .services(crate::core::runtime::ServiceSet::none())
        .domains(crate::core::runtime::DomainSet::none())
        .config(config)
        .extensions(extensions)
        .build()
        .await
        .expect("isolated core runtime");
    let ctx = crate::core::runtime::DispatchCtx::new(Arc::clone(runtime.context()));
    let envelope = envelope("esc-react-model-fallback");
    crate::core::bus::init().await.expect("bus init");

    runtime
        .run_in(
            Arc::clone(runtime.context()),
            crate::core::runtime::turn_origin::with_origin(
                crate::triage::local_trigger_origin(),
                apply_decision(
                    &ctx,
                    run_with_target(
                        TriageAction::React,
                        "trigger_reactor",
                        "handle this trigger with the parent model",
                    ),
                    &envelope,
                ),
            ),
        )
        .await
        .expect("react escalation should complete");

    let builds = builds.lock().unwrap();
    assert!(builds.iter().any(|(role, _)| role == "chat"));
    assert!(builds.iter().any(|(role, _)| role == "agentic"));
    drop(builds);
    let requests = requests.lock().unwrap();
    assert_eq!(
        requests.len(),
        1,
        "the hint model construction failure should fall back to the parent model"
    );
    assert_eq!(
        requests[0].0, "chat",
        "fallback must retain the parent model workload role"
    );
    assert_eq!(requests[0].1, TEST_MANAGED_MODEL);
    assert!(requests[0].2.messages.iter().any(|message| {
        message
            .text()
            .contains("handle this trigger with the parent model")
    }));
}

#[tokio::test]
async fn escalate_dispatch_preserves_orchestrator_tools_without_child_delegation() {
    let _events_guard = test_events_guard().await;
    let temp = tempfile::tempdir().expect("isolated triage workspace");
    let workspace = temp.path().join("workspace");
    let mut config = Config {
        workspace_dir: workspace.clone(),
        config_path: temp.path().join("config.toml"),
        action_dir: temp.path().join("actions"),
        ..Config::default()
    };
    config.default_model = Some("hint:coding".to_string());

    let requests = Arc::new(Mutex::new(Vec::new()));
    let builds = Arc::new(Mutex::new(Vec::new()));
    let extensions = crate::core::slots::Extensions::new();
    crate::inference::registry::register_chat_model_in(
        &extensions,
        crate::inference::provider::factory::PROVIDER_OPENHUMAN,
        triage_test_model(Arc::clone(&requests), Arc::clone(&builds), None),
    );
    let mut domains = crate::core::runtime::DomainSet::harness();
    domains.platform = true;
    let runtime = crate::core::runtime::CoreBuilder::new(crate::core::types::HostKind::Library)
        .services(crate::core::runtime::ServiceSet::none())
        .domains(domains)
        .tool_groups(crate::tools::toolpacks::ToolGroups::advertised())
        .config(config)
        .extensions(extensions)
        .build()
        .await
        .expect("isolated core runtime");
    let ctx = crate::core::runtime::DispatchCtx::new(Arc::clone(runtime.context()));
    let envelope = envelope("esc-inherit-model-role");
    crate::core::bus::init().await.expect("bus init");

    runtime
        .run_in(
            Arc::clone(runtime.context()),
            crate::core::runtime::turn_origin::with_origin(
                crate::triage::local_trigger_origin(),
                apply_decision(
                    &ctx,
                    run_with_target(
                        TriageAction::Escalate,
                        "orchestrator",
                        "route this task with the parent workload",
                    ),
                    &envelope,
                ),
            ),
        )
        .await
        .expect("escalation should complete");

    let requests = requests.lock().unwrap();
    assert_eq!(
        requests.len(),
        1,
        "the routed target should reach the model"
    );
    assert_eq!(
        requests[0].0, "coding",
        "an inherited target must keep its parent's workload role"
    );
    assert_eq!(requests[0].1, TEST_MANAGED_MODEL);
    assert!(requests[0].2.messages.iter().any(|message| {
        message
            .text()
            .contains("route this task with the parent workload")
    }));
    let tool_names: Vec<_> = requests[0]
        .2
        .tools
        .iter()
        .map(|tool| tool.name.as_str())
        .collect();
    assert!(
        tool_names.contains(&"grep"),
        "the orchestrator keeps its advertised coding tool"
    );
    assert!(!tool_names
        .iter()
        .any(|name| { *name == "spawn_subagent" || name.starts_with("delegate_") }));
}

#[tokio::test]
async fn react_dispatch_inherits_the_parent_workload_for_inherit_targets() {
    let _events_guard = test_events_guard().await;
    let temp = tempfile::tempdir().expect("isolated triage workspace");
    let workspace = temp.path().join("workspace");
    let mut config = Config {
        workspace_dir: workspace.clone(),
        config_path: temp.path().join("config.toml"),
        action_dir: temp.path().join("actions"),
        ..Config::default()
    };
    config.default_model = Some("hint:coding".to_string());
    let runtime_config = config.clone();

    let mut definitions = AgentDefinitionRegistry::builtins_only();
    let mut definition = definitions
        .get("trigger_reactor")
        .expect("built-in trigger reactor exists")
        .clone();
    definition.model = tinyagents_definition::ModelSpec::Inherit;
    definitions = definitions.with_definitions([definition]);
    let definitions = Arc::new(definitions);

    let requests = Arc::new(Mutex::new(Vec::new()));
    let builds = Arc::new(Mutex::new(Vec::new()));
    let extensions = crate::core::slots::Extensions::new();
    crate::inference::registry::register_chat_model_in(
        &extensions,
        crate::inference::provider::factory::PROVIDER_OPENHUMAN,
        triage_test_model(Arc::clone(&requests), Arc::clone(&builds), None),
    );
    let runtime = crate::core::runtime::CoreBuilder::new(crate::core::types::HostKind::Library)
        .services(crate::core::runtime::ServiceSet::none())
        .domains(crate::core::runtime::DomainSet::none())
        .config(config)
        .extensions(extensions)
        .build()
        .await
        .expect("isolated core runtime");
    let context = runtime.context().derive_with(
        crate::core::runtime::ContextOverlay::new(
            runtime_config,
            crate::core::runtime::DomainSet::none(),
            crate::tools::toolpacks::ToolGroups::none(),
        )
        .definitions(definitions),
    );
    let ctx = crate::core::runtime::DispatchCtx::new(Arc::clone(&context));
    let envelope = envelope("esc-inherit-workload");
    crate::core::bus::init().await.expect("bus init");

    runtime
        .run_in(
            context,
            crate::core::runtime::turn_origin::with_origin(
                crate::triage::local_trigger_origin(),
                apply_decision(
                    &ctx,
                    run_with_target(
                        TriageAction::Escalate,
                        "trigger_reactor",
                        "route this task with the inherited workload",
                    ),
                    &envelope,
                ),
            ),
        )
        .await
        .expect("escalation should complete");

    let requests = requests.lock().unwrap();
    assert_eq!(
        requests.len(),
        1,
        "the inherited target should reach the model"
    );
    assert_eq!(
        requests[0].0, "coding",
        "an Inherit target must retain its parent's workload role"
    );
    assert_eq!(requests[0].1, TEST_MANAGED_MODEL);
    assert!(requests[0].2.messages.iter().any(|message| {
        message
            .text()
            .contains("route this task with the inherited workload")
    }));
}
