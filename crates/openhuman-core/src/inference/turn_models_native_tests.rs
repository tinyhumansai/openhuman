use super::*;

#[tokio::test]
async fn native_turn_factory_applies_default_temperature_and_preserves_explicit_override() {
    struct TemperatureProbe(Arc<std::sync::Mutex<Vec<Option<f64>>>>);

    #[async_trait]
    impl ChatModel<()> for TemperatureProbe {
        async fn invoke(
            &self,
            _state: &(),
            request: ModelRequest,
        ) -> tinyinference_llm::Result<ModelResponse> {
            self.0.lock().unwrap().push(request.temperature);
            Ok(ModelResponse::assistant("ok"))
        }
    }

    let _guard = crate::inference::inference_test_guard_async().await;
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let _override = crate::inference::provider::factory::test_provider_override::install_model(
        Arc::new(TemperatureProbe(Arc::clone(&seen))),
    );
    let tmp = tempfile::tempdir().expect("isolated workspace");
    let config = Arc::new(managed_fixture_config(&tmp).await);
    let ctx = crate::core::runtime::DispatchCtx::unbound();
    let models = TurnModelSource::new_crate_native(&ctx, "chat", config)
        .build("hint:chat", 0.3, None, None)
        .await
        .expect("native model set");

    models
        .primary
        .invoke(
            &(),
            ModelRequest::new(vec![tinyinference_llm::message::Message::user("default")]),
        )
        .await
        .expect("default request");
    models
        .primary
        .invoke(
            &(),
            ModelRequest::new(vec![tinyinference_llm::message::Message::user("explicit")])
                .with_temperature(0.9),
        )
        .await
        .expect("explicit request");
    assert_eq!(*seen.lock().unwrap(), vec![Some(0.3), Some(0.9)]);
}
#[tokio::test]
async fn native_budget_wrapping_keeps_owner_error_slots_for_primary_tiers_and_summary() {
    use crate::agent::tinyagents::budget::{CallBudget, SpendLimits};
    use crate::core::runtime::{CoreContext, DispatchCtx, DomainSet};
    use crate::core::slots::Extensions;
    use tinyagents_harness::cost::ModelBudget;

    let _guard = crate::inference::inference_test_guard_async().await;
    let extensions = Extensions::new();
    crate::inference::registry::register_chat_model_in(
        &extensions,
        crate::inference::provider::factory::PROVIDER_OPENHUMAN,
        scoped_model_entry("budget-owner-model", false),
    );
    let context = CoreContext::for_test_with_host_kind_and_extensions(
        crate::core::types::HostKind::Library,
        DomainSet::full(),
        None,
        extensions,
    );
    let ctx = DispatchCtx::new(context.clone());
    let tmp = tempfile::tempdir().expect("isolated workspace");
    let config = Arc::new(managed_fixture_config(&tmp).await);
    let budget = ModelBudget::new(
        tinyinference_llm::model::budget::Budget::new(SpendLimits {
            tokens: Some(0),
            cost_micros: None,
        }),
        CallBudget {
            input_tokens: 10_000,
            output_tokens: 1,
            cost_micros: 1,
        },
    );
    let models = TurnModelSource::new_crate_native(&ctx, "chat", config)
        .build("hint:chat", 0.0, None, None)
        .await
        .expect("native model set")
        .with_host_controls(Some(&budget), None);
    let owner = models
        .native_model_set
        .as_ref()
        .expect("native owner set retained");
    let primary_slot = owner.primary_error_slot();
    let tier_slot = owner
        .workload_error_slot("hint:coding")
        .expect("coding route owns its error slot");
    let summary_slot = owner.summarizer_error_slot();
    let resolver = models.resolver();

    for (request, expected_slot) in [
        (
            ModelResolveRequest::new("chat").as_team_lead(),
            primary_slot,
        ),
        (
            ModelResolveRequest::new("worker").with_model_pin("hint:coding"),
            tier_slot,
        ),
    ] {
        let model = resolver.resolve(&request).await.expect("route resolves");
        let result = model
            .invoke(
                &(),
                ModelRequest::new(vec![tinyinference_llm::message::Message::user(
                    "budget this call",
                )]),
            )
            .await;
        let error = result.expect_err("zero token budget rejects each route");
        assert!(
            error.to_string().contains("budget exceeded"),
            "route must fail at its owner budget wrapper: {error}"
        );
        assert!(expected_slot.lock().unwrap().is_some());
    }

    let result = owner
        .summarizer()
        .invoke(
            &(),
            ModelRequest::new(vec![tinyinference_llm::message::Message::user(
                "budget this summary",
            )]),
        )
        .await;
    let error = result.expect_err("zero token budget rejects summary");
    assert!(
        error.to_string().contains("budget exceeded"),
        "summary must fail at its owner budget wrapper: {error}"
    );
    assert!(summary_slot.lock().unwrap().is_some());
}
#[tokio::test]
async fn explicit_provider_string_turn_source_uses_scoped_managed_model() {
    use crate::core::runtime::{CoreContext, DispatchCtx, DomainSet};
    use crate::core::slots::Extensions;
    use crate::core::types::HostKind;

    let provider = crate::inference::provider::factory::PROVIDER_OPENHUMAN;
    let explicit_extensions = Extensions::new();
    crate::inference::registry::register_chat_model_in(
        &explicit_extensions,
        provider,
        scoped_model_entry("explicit-provider-model", false),
    );
    let explicit_context = CoreContext::for_test_with_host_kind_and_extensions(
        HostKind::Library,
        DomainSet::full(),
        None,
        explicit_extensions,
    );
    let ambient_context =
        CoreContext::for_test_with_host_kind(HostKind::Library, DomainSet::full(), None);
    let dispatch = DispatchCtx::new(explicit_context);
    let tmp = tempfile::tempdir().expect("isolated workspace");
    let mut config = managed_fixture_config(&tmp).await;
    config.chat_provider = Some("unconfigured:must-not-win".into());

    CoreContext::scope(ambient_context, async {
        let source = TurnModelSource::new_crate_native_from_string(
            &dispatch,
            "chat",
            "openhuman",
            Arc::new(config),
        );
        let models = source
            .build("explicit-turn-model", 0.0, None, None)
            .await
            .expect("explicit provider string should construct through the scoped model");
        assert_eq!(name_of(&models.primary).await, "explicit-provider-model");
    })
    .await;
}

#[tokio::test]
async fn native_turn_preserves_deepseek_route_metadata_after_cloud_resolution() {
    use crate::config::schema::cloud_providers::{AuthStyle, CloudProviderCreds};
    use crate::core::runtime::{CoreContext, DispatchCtx, DomainSet};
    use crate::core::types::HostKind;

    let tmp = tempfile::tempdir().expect("isolated workspace");
    let config = crate::config::Config {
        config_path: tmp.path().join("config.toml"),
        workspace_dir: tmp.path().join("workspace"),
        chat_provider: Some("deepseek:hint:chat".into()),
        cloud_providers: vec![CloudProviderCreds {
            id: "deepseek-fixture".into(),
            slug: "deepseek".into(),
            label: "DeepSeek fixture".into(),
            endpoint: "https://api.deepseek.com/v1".into(),
            auth_style: AuthStyle::Bearer,
            default_model: Some("deepseek-chat".into()),
        }],
        ..Default::default()
    };
    crate::credentials::AuthService::from_config(&config)
        .store_provider_token(
            "provider:deepseek",
            "default",
            "sk-fixture-deepseek",
            Default::default(),
            true,
        )
        .await
        .expect("store isolated BYOK credential");
    let context = CoreContext::for_test_with_host_kind(
        HostKind::Library,
        DomainSet::full(),
        Some(tmp.path().join("library")),
    );
    let ctx = DispatchCtx::new(context);
    let models = TurnModelSource::new_crate_native(&ctx, "chat", Arc::new(config))
        .build("hint:chat", 0.2, None, None)
        .await
        .expect("configured DeepSeek turn model builds");

    assert_eq!(models.provider_id(), "deepseek");
    let route = models
        .native_model_set
        .as_ref()
        .expect("native owner route set")
        .primary_route();
    assert_eq!(route.provider, "deepseek");
    assert_eq!(route.model, "deepseek-chat");
    assert_eq!(route.route, "hint:chat");
    let profile = models.primary.profile().expect("cloud model profile");
    assert_eq!(profile.provider.as_deref(), Some("deepseek"));
    assert_eq!(profile.model.as_deref(), Some("deepseek-chat"));
}

#[tokio::test]
async fn injected_turn_profile_is_authoritative_and_skips_unselected_routes() {
    use crate::agent::host_overrides::HostOverrides;
    use crate::core::runtime::{ContextOverlay, CoreContext, DispatchCtx, DomainSet};
    use crate::core::slots::Extensions;
    use crate::core::types::HostKind;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tinyinference_llm::model::ModelProfile;

    struct ProfiledModel {
        profile: ModelProfile,
    }
    #[async_trait]
    impl ChatModel<()> for ProfiledModel {
        fn profile(&self) -> Option<&ModelProfile> {
            Some(&self.profile)
        }
        async fn invoke(
            &self,
            _state: &(),
            _request: ModelRequest,
        ) -> tinyinference_llm::Result<ModelResponse> {
            Ok(ModelResponse::assistant("injected"))
        }
    }

    let _guard = crate::inference::inference_test_guard_async().await;
    let builds = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&builds);
    let extensions = Extensions::new();
    crate::inference::registry::register_chat_model_in(
        &extensions,
        crate::inference::provider::factory::PROVIDER_OPENHUMAN,
        tinyinference_llm::providers::overrides::ChatModelEntry {
            build: Arc::new(move |_| {
                count.fetch_add(1, Ordering::SeqCst);
                Ok((Arc::new(NamedModel("unselected")), "unselected".into()))
            }),
            probe: None,
            models: None,
        },
    );
    let root = CoreContext::for_test_with_host_kind_and_extensions(
        HostKind::Library,
        DomainSet::full(),
        None,
        extensions,
    );
    let mut profile = ModelProfile::default();
    profile.provider = Some("ollama:injected".into());
    profile.tool_calling = false;
    profile.modalities.image_in = true;
    profile.max_input_tokens = Some(123_456);
    let mut overrides = HostOverrides::default();
    overrides.model = Some(Arc::new(ProfiledModel { profile }));
    let context = root.derive_with(
        ContextOverlay::new(
            crate::config::Config {
                chat_provider: Some("openhuman".into()),
                ..Default::default()
            },
            DomainSet::full(),
            Default::default(),
        )
        .host_overrides(Arc::new(overrides)),
    );
    let ctx = DispatchCtx::new(context);
    let models = TurnModelSource::new_crate_native(&ctx, "chat", Arc::new(Default::default()))
        .build("hint:chat", 0.2, None, None)
        .await
        .expect("host injected model builds");

    assert_eq!(models.provider_id(), "ollama:injected");
    assert!(!models.native_tools());
    assert!(models.supports_vision());
    assert!(models.is_local());
    assert_eq!(models.context_window(), Some(123_456));
    assert_eq!(builds.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn injected_turn_without_profile_keeps_conservative_capabilities() {
    use crate::agent::host_overrides::HostOverrides;
    use crate::core::runtime::{ContextOverlay, CoreContext, DispatchCtx, DomainSet};
    use crate::core::types::HostKind;

    struct ProfilelessModel;
    #[async_trait]
    impl ChatModel<()> for ProfilelessModel {
        async fn invoke(
            &self,
            _state: &(),
            _request: ModelRequest,
        ) -> tinyinference_llm::Result<ModelResponse> {
            Ok(ModelResponse::assistant("profileless"))
        }
    }

    let root = CoreContext::for_test_with_host_kind(HostKind::Library, DomainSet::full(), None);
    let mut overrides = HostOverrides::default();
    overrides.model = Some(Arc::new(ProfilelessModel));
    let context = root.derive_with(
        ContextOverlay::new(
            crate::config::Config {
                chat_provider: Some("openhuman".into()),
                ..Default::default()
            },
            DomainSet::full(),
            Default::default(),
        )
        .host_overrides(Arc::new(overrides)),
    );
    let ctx = DispatchCtx::new(context.clone());
    let models = CoreContext::scope(context, async {
        TurnModelSource::new_crate_native(&ctx, "chat", Arc::new(Default::default()))
            .build("hint:chat", 0.2, None, None)
            .await
            .expect("profileless host-injected model builds")
    })
    .await;

    assert_eq!(models.provider_id(), "injected");
    assert!(!models.native_tools());
    assert!(!models.supports_vision());
    assert!(!models.is_local());
    assert_eq!(models.context_window(), None);
    assert!(models.primary.profile().is_none());
}
