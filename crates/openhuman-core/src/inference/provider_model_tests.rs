use super::*;
use tinyinference_llm::providers::overrides::{ChatModelEntry, ProviderModelMetadata};

#[test]
fn absent_provider_keeps_default_absent_and_concrete_model_ids_verbatim() {
    assert_eq!(
        crate::inference::registry::provider_default_model("missing-provider"),
        None
    );
    let config = Config::default();
    assert_eq!(
        resolve_provider_model_hint("missing-provider", "vendor/model-id", &config),
        "vendor/model-id"
    );
}

#[test]
fn abstract_hint_markers_do_not_become_raw_model_ids() {
    assert!(is_abstract_model("missing-provider", "hint:unknown"));
    assert!(!is_raw_model("missing-provider", "hint:unknown"));
    assert!(is_raw_model("missing-provider", "vendor/model-id"));
    assert!(is_raw_model("missing-provider", "chat"));
    assert!(is_raw_model("missing-provider", "coding"));
    assert_eq!(
        role_for_provider_model("missing-provider", "hint:coding"),
        "coding"
    );
    assert_eq!(
        role_for_provider_model("missing-provider", "hint:unknown"),
        "chat"
    );
}

#[test]
fn registered_model_defaults_and_aliases_are_context_scoped() {
    use crate::core::slots::Extensions;
    use crate::inference::registry::register_chat_model_in;
    use std::sync::Arc;

    let extensions = Extensions::new();
    register_chat_model_in(
        &extensions,
        crate::inference::provider::factory::PROVIDER_OPENHUMAN,
        ChatModelEntry {
            build: Arc::new(|_| panic!("metadata-only provider entry")),
            probe: None,
            models: Some(ProviderModelMetadata {
                default_model: Arc::new(|| Some("scoped/model".to_string())),
                role_for_model: Arc::new(|model| {
                    (model == "scoped-alias").then(|| "reasoning".to_string())
                }),
                is_alias: Arc::new(|model| model == "scoped-alias"),
                is_raw_model: Arc::new(|model| model == "scoped/model"),
                supports_vision: Arc::new(|_| false),
                resolve_model: Arc::new(|role, _, override_model| {
                    (override_model == Some("scoped-alias")).then(|| format!("scoped-{role}"))
                }),
                context_window: Arc::new(|model| match model {
                    "" => Some(99_999),
                    "scoped/model" => Some(77_777),
                    "hint:reasoning" => Some(88_888),
                    _ => None,
                }),
                default_pricing: Arc::new(|| None),
                without_reasoning: Arc::new(|request| request),
            }),
        },
    );
    assert_eq!(
        crate::inference::registry::provider_default_model_for(
            Some(&extensions),
            crate::inference::provider::factory::PROVIDER_OPENHUMAN
        )
        .as_deref(),
        Some("scoped/model")
    );
    assert_eq!(
        crate::inference::context_window::project_context_window_for_model(
            "scoped/model",
            Some(Some(&extensions))
        )
        .window,
        Some(77_777)
    );
    assert_eq!(
        crate::inference::context_window::project_context_window_for_model(
            "hint:reasoning",
            Some(Some(&extensions))
        )
        .window,
        Some(88_888),
        "a registered provider hook can override a workload tier alias"
    );
    assert_eq!(
        crate::inference::context_window::project_context_window_for_model(
            "   ",
            Some(Some(&extensions))
        )
        .window,
        None,
        "blank model IDs do not query provider hooks or the catalog"
    );
    let context = crate::core::runtime::CoreContext::for_test_with_extensions(
        crate::core::runtime::DomainSet::full(),
        None,
        extensions.clone(),
    );

    assert_eq!(
        crate::inference::registry::provider_default_model(
            crate::inference::provider::factory::PROVIDER_OPENHUMAN
        ),
        None
    );
    crate::core::runtime::CoreContext::sync_scope(context, || {
        assert_eq!(
            crate::inference::context_window::project_context_window_for_model(
                "scoped/model",
                None
            )
            .window,
            Some(77_777),
            "the ambient projection uses the active provider registry"
        );
        assert_ne!(
            crate::inference::context_window::project_context_window_for_model(
                "scoped/model",
                Some(None)
            )
            .window,
            Some(77_777),
            "an explicit process-only projection does not borrow ambient context"
        );
        let empty_extensions = Extensions::new();
        assert_ne!(
            crate::inference::context_window::project_context_window_for_model(
                "scoped/model",
                Some(Some(&empty_extensions))
            )
            .window,
            Some(77_777),
            "an explicit empty context does not borrow the ambient provider registration"
        );
        assert_eq!(
            crate::inference::registry::provider_default_model(
                crate::inference::provider::factory::PROVIDER_OPENHUMAN
            )
            .as_deref(),
            Some("scoped/model")
        );
        assert_eq!(
            crate::inference::registry::provider_model_role(
                crate::inference::provider::factory::PROVIDER_OPENHUMAN,
                "scoped-alias"
            )
            .as_deref(),
            Some("reasoning")
        );
        assert_eq!(
            role_for_provider_model(
                crate::inference::provider::factory::PROVIDER_OPENHUMAN,
                "scoped-alias"
            ),
            "reasoning",
            "the public resolver must use the ambient context registration"
        );
        assert_eq!(
            role_for_provider_model_for(
                Some(&extensions),
                crate::inference::provider::factory::PROVIDER_OPENHUMAN,
                "scoped-alias",
            ),
            "reasoning",
            "an explicit context must use that context's provider hooks"
        );
        assert_eq!(
            role_for_provider_model_for(
                None,
                crate::inference::provider::factory::PROVIDER_OPENHUMAN,
                "scoped-alias",
            ),
            "chat",
            "explicit process-only lookup must not borrow the ambient provider hook"
        );
        assert_eq!(
            resolve_provider_model_hint(
                crate::inference::provider::factory::PROVIDER_OPENHUMAN,
                "scoped-alias",
                &Config::default(),
            ),
            "scoped-reasoning",
            "the public hint resolver must preserve ambient route and model hooks"
        );
        assert_eq!(
            resolve_provider_model_hint_for(
                None,
                crate::inference::provider::factory::PROVIDER_OPENHUMAN,
                "scoped-alias",
                &Config::default(),
            ),
            "scoped-alias",
            "explicit process-only hint resolution must not borrow ambient hooks"
        );
        assert_eq!(
            resolve_provider_model_hint_for(
                Some(&extensions),
                crate::inference::provider::factory::PROVIDER_OPENHUMAN,
                "scoped-alias",
                &Config::default(),
            ),
            "scoped-reasoning",
            "explicit context hint resolution must use that context's hooks"
        );
    });
    assert_eq!(
        crate::inference::registry::provider_default_model(
            crate::inference::provider::factory::PROVIDER_OPENHUMAN
        ),
        None
    );
}

#[test]
fn native_provider_metadata_resolves_by_context_and_preserves_request_fields() {
    use crate::core::slots::Extensions;
    use crate::inference::registry::{
        register_chat_model, register_chat_model_in, unregister_chat_model, ProviderContext,
    };
    use serde_json::json;
    use std::sync::Arc;

    fn entry(label: &'static str, window: u64) -> ChatModelEntry<ProviderContext> {
        ChatModelEntry {
            build: Arc::new(|_| panic!("metadata lookup must not build a model")),
            probe: None,
            models: Some(ProviderModelMetadata {
                default_model: Arc::new(move || Some(format!("{label}/default"))),
                role_for_model: Arc::new(move |model| {
                    (model == format!("{label}/alias")).then(|| format!("{label}/role"))
                }),
                is_alias: Arc::new(move |model| model == format!("{label}/alias")),
                is_raw_model: Arc::new(move |model| model == format!("{label}/raw")),
                supports_vision: Arc::new(move |model| model == format!("{label}/vision")),
                resolve_model: Arc::new(move |role, context: &ProviderContext, pin| {
                    Some(format!(
                        "{label}:{}:{}",
                        context.config.default_model.as_deref().unwrap_or("unset"),
                        pin.unwrap_or(role)
                    ))
                }),
                context_window: Arc::new(move |model| {
                    (model == format!("{label}/raw")).then_some(window)
                }),
                default_pricing: Arc::new(move || Some((format!("{label}/priced"), 1.25, 2.5))),
                without_reasoning: Arc::new(move |mut request| {
                    request.metadata["reasoning_removed_by"] = json!(label);
                    request
                }),
            }),
        }
    }

    let _guard = crate::inference::inference_test_guard();
    let provider = "provider-metadata-scope-regression";
    let process = entry("process", 101);
    register_chat_model(provider, process);
    let a_extensions = Extensions::new();
    register_chat_model_in(&a_extensions, provider, entry("context-a", 202));
    let b_extensions = Extensions::new();
    register_chat_model_in(&b_extensions, provider, entry("context-b", 303));

    let verify = |extensions: Option<&Extensions>, label: &str, window: u64| {
        assert_eq!(
            crate::inference::registry::provider_default_model_for(extensions, provider).as_deref(),
            Some(format!("{label}/default").as_str())
        );
        assert_eq!(
            crate::inference::registry::provider_model_role_for(
                extensions,
                provider,
                &format!("{label}/alias")
            )
            .as_deref(),
            Some(format!("{label}/role").as_str())
        );
        assert!(crate::inference::registry::provider_model_is_alias_for(
            extensions,
            provider,
            &format!("{label}/alias")
        ));
        assert!(
            crate::inference::registry::provider_model_supports_vision_for(
                extensions,
                provider,
                &format!("{label}/vision")
            )
        );
        let config = Config {
            default_model: Some(format!("{label}/config")),
            ..Config::default()
        };
        assert_eq!(
            crate::inference::registry::resolve_provider_model_for(
                extensions,
                provider,
                "chat",
                &config,
                Some("pin")
            ),
            Some(format!("{label}:{label}/config:pin"))
        );
        assert_eq!(
            crate::inference::registry::provider_model_context_window_for(
                extensions,
                provider,
                &format!("{label}/raw")
            ),
            Some(window)
        );
        let mut request = tinyinference_llm::model::ModelRequest::new(vec![]);
        request.model = Some(format!("{label}/raw"));
        request.max_tokens = Some(37);
        request.metadata = json!({"caller": "kept"});
        let ambient_extensions = extensions.cloned().unwrap_or_else(Extensions::new);
        let ambient = crate::core::runtime::CoreContext::for_test_with_extensions(
            crate::core::runtime::DomainSet::full(),
            None,
            ambient_extensions,
        );
        crate::core::runtime::CoreContext::sync_scope(ambient, || {
            assert!(crate::inference::registry::provider_model_is_raw(
                provider,
                &format!("{label}/raw")
            ));
            assert_eq!(
                crate::inference::registry::provider_default_pricing(provider),
                Some((format!("{label}/priced"), 1.25, 2.5))
            );
            let mut request = tinyinference_llm::model::ModelRequest::new(vec![]);
            request.model = Some(format!("{label}/raw"));
            request.max_tokens = Some(37);
            request.metadata = json!({"caller": "kept"});
            let request =
                crate::inference::registry::provider_request_without_reasoning(provider, request);
            assert_eq!(
                request.model.as_deref(),
                Some(format!("{label}/raw").as_str())
            );
            assert_eq!(request.max_tokens, Some(37));
            assert_eq!(
                request.metadata,
                json!({"caller": "kept", "reasoning_removed_by": label})
            );
        });
    };

    verify(None, "process", 101);
    let empty_extensions = Extensions::new();
    verify(Some(&empty_extensions), "process", 101);
    verify(Some(&a_extensions), "context-a", 202);
    verify(Some(&b_extensions), "context-b", 303);

    let ambient_a = crate::core::runtime::CoreContext::for_test_with_extensions(
        crate::core::runtime::DomainSet::full(),
        None,
        a_extensions.clone(),
    );
    crate::core::runtime::CoreContext::sync_scope(ambient_a, || {
        assert_eq!(
            crate::inference::registry::provider_default_model(provider).as_deref(),
            Some("context-a/default")
        );
        assert_eq!(
            crate::inference::registry::provider_default_pricing(provider)
                .as_ref()
                .map(|price| price.0.as_str()),
            Some("context-a/priced")
        );
        let request = crate::inference::registry::provider_request_without_reasoning(
            provider,
            tinyinference_llm::model::ModelRequest::new(vec![]),
        );
        assert_eq!(request.metadata["reasoning_removed_by"], "context-a");
        assert_eq!(
            crate::inference::registry::provider_model_context_window_for(
                None,
                provider,
                "process/raw"
            ),
            Some(101),
            "explicit process-only lookup must ignore ambient registration"
        );
    });
    unregister_chat_model(provider);
}

#[test]
fn managed_builder_uses_the_entry_selected_before_metadata_resolution() {
    use crate::core::slots::Extensions;
    use crate::inference::registry::{register_chat_model_in, ProviderContext};
    use std::sync::Arc;

    let _guard = crate::inference::inference_test_guard();
    let provider = crate::inference::provider::factory::PROVIDER_OPENHUMAN;
    let extensions = Extensions::new();
    let replacement = ChatModelEntry::<ProviderContext> {
        build: Arc::new(|_| {
            Ok((
                Arc::new(tinyagents_harness::testkit::ScriptedModel::replies(vec![
                    "replacement",
                ])),
                "replacement-factory".to_string(),
            ))
        }),
        probe: None,
        models: None,
    };
    let replacement_for_resolver = replacement.clone();
    let extensions_for_resolver = extensions.clone();
    let entry = ChatModelEntry::<ProviderContext> {
        build: Arc::new(|_| {
            Ok((
                Arc::new(tinyagents_harness::testkit::ScriptedModel::replies(vec![
                    "selected",
                ])),
                "selected-factory".to_string(),
            ))
        }),
        probe: None,
        models: Some(ProviderModelMetadata {
            default_model: Arc::new(|| None),
            role_for_model: Arc::new(|_| None),
            is_alias: Arc::new(|_| false),
            is_raw_model: Arc::new(|_| false),
            supports_vision: Arc::new(|_| false),
            resolve_model: Arc::new(move |_, _, _| {
                register_chat_model_in(
                    &extensions_for_resolver,
                    provider,
                    replacement_for_resolver.clone(),
                );
                Some("snapshot/model".to_string())
            }),
            context_window: Arc::new(|_| None),
            default_pricing: Arc::new(|| None),
            without_reasoning: Arc::new(|request| request),
        }),
    };
    register_chat_model_in(&extensions, provider, entry);

    let built = crate::inference::provider::factory::make_openhuman_backend_model_for_thread_in(
        Some(&extensions),
        "chat",
        &Config::default(),
        None,
        true,
        None,
        &crate::security::egress::EgressLedger::none(),
    )
    .expect("the selected entry must remain usable after its registry slot changes");
    assert_eq!(built.1, "selected-factory");
}
