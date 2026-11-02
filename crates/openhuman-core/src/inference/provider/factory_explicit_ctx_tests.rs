use super::*;
use tinyinference_llm::providers::overrides::{
    CatalogEntry, ChatModelEntry, ProviderModelMetadata,
};

#[tokio::test]
async fn managed_factory_and_probe_use_the_explicit_provider_entry() {
    use crate::core::runtime::{CoreContext, DispatchCtx, DomainSet};
    use crate::core::slots::Extensions;
    use crate::inference::registry::ProviderContext;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use tinyinference_llm::message::Message;
    use tinyinference_llm::model::ModelRequest;

    fn entry(name: &'static str, probed: Arc<AtomicBool>) -> ChatModelEntry<ProviderContext> {
        ChatModelEntry {
            build: Arc::new(move |_| {
                Ok((
                    Arc::new(tinyagents_harness::testkit::ScriptedModel::replies(vec![
                        name,
                    ])),
                    name.to_string(),
                ))
            }),
            probe: Some(Arc::new(move |request| {
                let actual = probed.clone();
                Box::pin(async move {
                    assert_eq!(request.model, name);
                    actual.store(true, Ordering::SeqCst);
                    Ok(())
                })
            })),
            models: Some(ProviderModelMetadata {
                default_model: Arc::new(move || Some(name.to_string())),
                role_for_model: Arc::new(|_| None),
                is_alias: Arc::new(|_| false),
                is_raw_model: Arc::new(|model| !model.trim().is_empty()),
                supports_vision: Arc::new(|_| false),
                resolve_model: Arc::new(move |_, _, _| Some(name.to_string())),
                context_window: Arc::new(|_| None),
                default_pricing: Arc::new(|| None),
                without_reasoning: Arc::new(|request| request),
            }),
        }
    }

    let provider = crate::inference::provider::factory::PROVIDER_OPENHUMAN;
    let _guard = crate::inference::inference_test_guard_async().await;
    let a_probed = Arc::new(AtomicBool::new(false));
    let b_probed = Arc::new(AtomicBool::new(false));
    let a_extensions = Extensions::new();
    crate::inference::registry::register_chat_model_in(
        &a_extensions,
        provider,
        entry("context-a-one-shot", Arc::clone(&a_probed)),
    );
    let context_a = CoreContext::for_test_with_host_kind_and_extensions(
        crate::core::types::HostKind::Library,
        DomainSet::full(),
        None,
        a_extensions,
    );
    let b_extensions = Extensions::new();
    crate::inference::registry::register_chat_model_in(
        &b_extensions,
        provider,
        entry("context-b-one-shot", Arc::clone(&b_probed)),
    );
    let context_b = CoreContext::for_test_with_host_kind_and_extensions(
        crate::core::types::HostKind::Library,
        DomainSet::full(),
        None,
        b_extensions,
    );
    let dispatch = DispatchCtx::new(context_a);
    let tmp = tempfile::tempdir().expect("isolated workspace");
    let config = Config {
        config_path: tmp.path().join("config.toml"),
        workspace_dir: tmp.path().join("workspace"),
        ..Default::default()
    };
    crate::credentials::AuthService::from_config(&config)
        .store_provider_token(
            crate::credentials::APP_SESSION_PROVIDER,
            "default",
            "fixture-session-token",
            Default::default(),
            true,
        )
        .await
        .expect("store isolated fixture session");

    CoreContext::scope(context_b, async {
        let (_, model) = create_chat_model_with_model_id_in(&dispatch, "chat", &config, 0.0)
            .await
            .expect("explicit model entry builds");
        assert_eq!(model, "context-a-one-shot");
        let simple_chat = crate::inference::host_runtime::ops::agent_chat_simple(
            &dispatch,
            &config,
            "fixture prompt",
            None,
            None,
            None,
        )
        .await
        .expect("simple chat uses its explicit model entry");
        assert_eq!(simple_chat.value, "context-a-one-shot");
        let turn = crate::inference::turn_models::TurnModelSource::new_crate_native(
            &dispatch,
            "chat",
            Arc::new(config.clone()),
        )
        .build("hint:chat", 0.0, None, None)
        .await
        .expect("turn set uses explicit model entry");
        let response = turn
            .primary
            .invoke(&(), ModelRequest::new(vec![Message::user("probe")]))
            .await
            .expect("fixture response");
        assert_eq!(response.text(), "context-a-one-shot");
        probe_inference_readiness(&dispatch, "chat", &config)
            .await
            .expect("explicit model entry is ready");
        let missing = DispatchCtx::new(CoreContext::for_test(DomainSet::full(), None));
        assert!(
            create_chat_model_with_model_id_in(&missing, "chat", &config, 0.0)
                .await
                .is_err()
        );
    })
    .await;
    assert!(a_probed.load(Ordering::SeqCst));
    assert!(!b_probed.load(Ordering::SeqCst));
}

#[tokio::test]
async fn hosted_cloud_slug_uses_explicit_auth_policy_and_model_entry() {
    use crate::config::schema::cloud_providers::{
        register_hosted_cloud_provider_policy_in, AuthStyle, CloudProviderCreds,
    };
    use crate::core::runtime::{CoreContext, DispatchCtx, DomainSet};
    use crate::core::slots::Extensions;
    use crate::inference::registry::{register_catalog_in, ProviderContext};
    use std::sync::Arc;
    use tinyinference_llm::message::Message;
    use tinyinference_llm::model::ModelRequest;

    fn entry(name: &'static str) -> ChatModelEntry<ProviderContext> {
        ChatModelEntry {
            build: Arc::new(move |_| {
                Ok((
                    Arc::new(tinyagents_harness::testkit::ScriptedModel::replies(vec![
                        name,
                    ])),
                    name.to_string(),
                ))
            }),
            probe: None,
            models: Some(ProviderModelMetadata {
                default_model: Arc::new(move || Some(name.to_string())),
                role_for_model: Arc::new(|_| None),
                is_alias: Arc::new(|_| false),
                is_raw_model: Arc::new(|model| !model.trim().is_empty()),
                supports_vision: Arc::new(|_| false),
                resolve_model: Arc::new(move |_, _, _| Some(name.to_string())),
                context_window: Arc::new(|_| None),
                default_pricing: Arc::new(|| None),
                without_reasoning: Arc::new(|request| request),
            }),
        }
    }

    fn catalog(name: &'static str) -> CatalogEntry<ProviderContext> {
        CatalogEntry {
            list: Arc::new(|_| {
                Box::pin(async { Err("model listing is not used by this test".to_string()) })
            }),
            limits: Some(Arc::new(move |request| {
                Box::pin(async move {
                    Some(tinyinference_llm::model::discover::DiscoveryRequest {
                        endpoint: format!("https://{name}.example.test/v1"),
                        model: request.model,
                        listing_url: None,
                        probe_single_model: false,
                        headers: Vec::new(),
                        pinned_providers: Vec::new(),
                        ollama_native: None,
                        timeout: std::time::Duration::from_secs(1),
                    })
                })
            })),
        }
    }

    async fn response(model: Arc<dyn tinyinference_llm::model::ChatModel<()>>) -> String {
        model
            .invoke(&(), ModelRequest::new(vec![Message::user("fixture")]))
            .await
            .expect("scripted hosted model response")
            .text()
    }

    let _guard = crate::inference::inference_test_guard_async().await;
    let provider = crate::inference::provider::factory::PROVIDER_OPENHUMAN;
    let mut outcomes = Vec::new();
    for ambient_owns_auth in [false, true] {
        let a_extensions = Extensions::new();
        crate::inference::registry::register_chat_model_in(
            &a_extensions,
            provider,
            entry("context-a-hosted-slug"),
        );
        register_catalog_in(&a_extensions, provider, catalog("context-a-catalog"));
        register_hosted_cloud_provider_policy_in(
            &a_extensions,
            Arc::new(|slug| slug == "p_hosted"),
            Arc::new(|style| style == AuthStyle::OpenhumanJwt),
        );
        let context_a = CoreContext::for_test_with_host_kind_and_extensions(
            crate::core::types::HostKind::Library,
            DomainSet::full(),
            None,
            a_extensions,
        );

        let b_extensions = Extensions::new();
        crate::inference::registry::register_chat_model_in(
            &b_extensions,
            provider,
            entry("context-b-hosted-slug"),
        );
        register_catalog_in(&b_extensions, provider, catalog("context-b-catalog"));
        register_hosted_cloud_provider_policy_in(
            &b_extensions,
            Arc::new(move |slug| ambient_owns_auth && slug == "p_hosted"),
            Arc::new(move |style| ambient_owns_auth && style == AuthStyle::OpenhumanJwt),
        );
        let context_b = CoreContext::for_test_with_host_kind_and_extensions(
            crate::core::types::HostKind::Library,
            DomainSet::full(),
            None,
            b_extensions,
        );

        let dispatch = DispatchCtx::new(Arc::clone(&context_a));
        let tmp = tempfile::tempdir().expect("isolated workspace");
        let mut config = Config {
            config_path: tmp.path().join("config.toml"),
            workspace_dir: tmp.path().join("workspace"),
            ..Default::default()
        };
        crate::credentials::AuthService::from_config(&config)
            .store_provider_token(
                crate::credentials::APP_SESSION_PROVIDER,
                "default",
                "fixture-session-token",
                Default::default(),
                true,
            )
            .await
            .expect("store isolated fixture session");
        config.chat_provider = Some("p_hosted:fixture/model-v1".to_string());
        config.cloud_providers.push(CloudProviderCreds {
            id: "p_hosted_fixture".to_string(),
            slug: "p_hosted".to_string(),
            label: "Hosted fixture".to_string(),
            endpoint: "https://hosted.example.test/v1".to_string(),
            auth_style: AuthStyle::OpenhumanJwt,
            default_model: Some("fixture/model-v1".to_string()),
        });

        let observed = CoreContext::scope(context_b, async {
            let resolved = dispatch.resolved().expect("bound dispatch context");
            let request = crate::inference::context_window::model_limits_request_for(
                "chat",
                "openhuman",
                "fixture/model-v1",
                &config,
                Some(resolved.extensions()),
            )
            .await
            .expect("explicit context catalog supplies limits");
            let one_shot = create_chat_model_with_model_id_in(&dispatch, "chat", &config, 0.0)
                .await
                .map(|(_, model)| model)
                .unwrap_or_else(|error| format!("ERR: {error}"));
            let turn = crate::inference::turn_models::TurnModelSource::new_crate_native(
                &dispatch,
                "chat",
                Arc::new(config.clone()),
            )
            .build("hint:chat", 0.0, None, None)
            .await
            .ok()
            .map(|models| models.primary);
            let turn_response = match turn {
                Some(model) => response(model).await,
                None => "ERR: turn set construction failed".to_string(),
            };
            (one_shot, turn_response, request.endpoint)
        })
        .await;
        let explicit_string = CoreContext::scope(Arc::clone(&context_a), async {
            let (model, model_id) = create_chat_model_from_string_with_model_id(
                "chat",
                "p_hosted:fixture/model-v1",
                &config,
                0.0,
            )
            .await
            .expect("explicit hosted cloud string builds from the ambient owner context");
            (model_id, response(model).await)
        })
        .await;
        outcomes.push((observed, explicit_string));
    }

    assert_eq!(
        outcomes,
        vec![
            (
                (
                    "context-a-hosted-slug".to_string(),
                    "context-a-hosted-slug".to_string(),
                    "https://context-a-catalog.example.test/v1".to_string(),
                ),
                (
                    "context-a-hosted-slug".to_string(),
                    "context-a-hosted-slug".to_string(),
                ),
            ),
            (
                (
                    "context-a-hosted-slug".to_string(),
                    "context-a-hosted-slug".to_string(),
                    "https://context-a-catalog.example.test/v1".to_string(),
                ),
                (
                    "context-a-hosted-slug".to_string(),
                    "context-a-hosted-slug".to_string(),
                ),
            ),
        ]
    );
}

/// A flow's LLM capability builds its model under the context it was handed,
/// not the calling task's: the host override of the explicit context wins over
/// an installed ambient model, and two contexts never see each other's model.
#[tokio::test]
async fn create_chat_model_in_reads_the_models_of_the_explicit_context() {
    use crate::agent::host_overrides::HostOverrides;
    use crate::core::runtime::{ContextOverlay, CoreContext, DispatchCtx, DomainSet};
    use std::sync::Arc;
    use tinyagents_harness::testkit::ScriptedModel;
    use tinyinference_llm::message::Message;
    use tinyinference_llm::model::ModelRequest;

    let _guard = crate::inference::inference_test_guard_async().await;
    let _override = test_provider_override::install_model(Arc::new(ScriptedModel::replies(vec![
        "echo: ambient",
    ])));
    let config = Config::default();
    let parent = CoreContext::for_test(DomainSet::full(), None);
    let ctx_for = |reply: &'static str| {
        let mut overrides = HostOverrides::default();
        overrides.model = Some(Arc::new(ScriptedModel::replies(vec![reply])));
        let mut overlay = ContextOverlay::new(
            config.clone(),
            DomainSet::full(),
            crate::tools::toolpacks::ToolGroups::default(),
        );
        overlay.host_overrides = Some(Arc::new(overrides));
        DispatchCtx::new(parent.derive_with(overlay))
    };

    for reply in ["echo: one", "echo: two"] {
        let ctx = ctx_for(reply);
        let model = create_chat_model_in(&ctx, "chat", &config, 0.3)
            .await
            .expect("explicit model");
        let response = model
            .invoke(&(), ModelRequest::new(vec![Message::user("hi")]))
            .await
            .expect("invoke must succeed");
        assert_eq!(response.text(), reply);
    }
    probe_inference_readiness(&ctx_for("echo: probe"), "chat", &config)
        .await
        .expect("the explicit context's model is ready");
}
