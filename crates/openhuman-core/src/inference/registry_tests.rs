use super::*;
use crate::inference::context_window::model_limits_request;
use crate::inference::provider::factory::{
    create_chat_model_from_string, create_chat_model_with_identity_in_for_inspection,
    make_openhuman_backend_model_for_thread_in, PROVIDER_OPENHUMAN,
};
use async_trait::async_trait;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use tinyinference_llm::model::discover::DiscoveryRequest;
use tinyinference_llm::model::ChatModel;
use tinyinference_llm::model::{ModelRequest, ModelResponse};
use tinyinference_llm::providers::overrides::CatalogRequest;
use tinyinference_llm::providers::overrides::{
    CatalogEntry, ChatModelEntry, ProviderModelMetadata,
};

type SeenModelRequests = Arc<Mutex<Vec<(String, Option<String>, Option<String>)>>>;

struct FakeModel;

#[async_trait]
impl ChatModel<()> for FakeModel {
    async fn invoke(
        &self,
        _state: &(),
        _request: ModelRequest,
    ) -> tinyinference_llm::Result<ModelResponse> {
        Ok(ModelResponse::assistant("from the registered model"))
    }
}

fn fake_entry(seen: SeenModelRequests) -> ChatModelEntry<ProviderContext> {
    ChatModelEntry {
        build: Arc::new(move |request| {
            seen.lock().unwrap().push((
                request.role.to_string(),
                request.model_override.map(str::to_string),
                request.thread_id.map(str::to_string),
            ));
            Ok((
                Arc::new(FakeModel) as Arc<dyn ChatModel<()>>,
                "fake-model".to_string(),
            ))
        }),
        probe: None,
        models: Some(ProviderModelMetadata {
            default_model: Arc::new(|| Some("host/fake-default".to_string())),
            role_for_model: Arc::new(|model| {
                (model == "hint:reasoning").then(|| "reasoning".to_string())
            }),
            is_alias: Arc::new(|model| model == "hint:reasoning"),
            is_raw_model: Arc::new(|model| model != "hint:reasoning"),
            supports_vision: Arc::new(|_| false),
            resolve_model: Arc::new(|_, config, model| match model {
                Some("hint:reasoning") => Some("host/resolved-reasoning".to_string()),
                Some(model) if !model.starts_with("hint:") => Some(model.to_string()),
                Some(_) => None,
                None => config
                    .config
                    .default_model
                    .clone()
                    .or_else(|| Some("host/fake-default".to_string())),
            }),
            context_window: Arc::new(|_| None),
            default_pricing: Arc::new(|| None),
            without_reasoning: Arc::new(|request| request),
        }),
    }
}

#[test]
fn nothing_registered_resolves_no_provider() {
    let _guard = crate::inference::inference_test_guard();
    assert!(chat_model(PROVIDER_OPENHUMAN).is_none());
    assert!(embedding(PROVIDER_OPENHUMAN).is_none());
    assert!(catalog(PROVIDER_OPENHUMAN).is_none());
}

#[tokio::test]
async fn a_registered_chat_model_serves_the_managed_provider_for_a_thread() {
    let _guard = crate::inference::inference_test_guard_async().await;
    let _exclusive = crate::inference::provider::factory::test_provider_override::exclusive();
    let seen = Arc::default();
    register_chat_model(PROVIDER_OPENHUMAN, fake_entry(Arc::clone(&seen)));

    let built = make_openhuman_backend_model_for_thread_in(
        None,
        "chat",
        &Config::default(),
        Some("hint:reasoning"),
        true,
        Some("thread-1"),
        &EgressLedger::none(),
    );
    let library = crate::core::runtime::CoreContext::for_test_with_host_kind(
        crate::core::types::HostKind::Library,
        crate::core::runtime::DomainSet::full(),
        None,
    );
    let one_shot = crate::core::runtime::CoreContext::scope(library, async {
        create_chat_model_from_string("chat", PROVIDER_OPENHUMAN, &Config::default(), 0.5).await
    })
    .await;
    unregister_chat_model(PROVIDER_OPENHUMAN);

    let (model, resolved) = built.expect("registered model builds");
    assert_eq!(resolved, "fake-model");
    let reply = model
        .invoke(&(), ModelRequest::new(vec![]))
        .await
        .expect("invoke");
    assert_eq!(reply.text(), "from the registered model");
    assert!(one_shot.is_ok());
    let seen = seen.lock().unwrap();
    assert_eq!(
        seen[0],
        (
            "chat".to_string(),
            Some("host/resolved-reasoning".to_string()),
            Some("thread-1".to_string())
        )
    );
    assert_eq!(seen[1].1.as_deref(), Some("host/fake-default"));
    assert_eq!(
        seen.len(),
        2,
        "both construction paths consulted the registry"
    );
}

#[tokio::test]
async fn a_registered_embedding_factory_builds_the_managed_embedding_model() {
    let _guard = crate::inference::inference_test_guard();
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&calls);
    register_embedding(
        PROVIDER_OPENHUMAN,
        Arc::new(move |request| {
            let counter = Arc::clone(&counter);
            Box::pin(async move {
                assert_eq!(request.dimensions, 8);
                counter.fetch_add(1, Ordering::SeqCst);
                Ok(Box::new(tinyinference_embeddings::NoopEmbeddingModel)
                    as Box<dyn tinyinference_embeddings::EmbeddingModel>)
            })
        }),
    );
    let built = crate::inference::embedding_host::create_embedding_provider_with_credentials(
        "cloud",
        "any-model",
        8,
        "",
        None,
    )
    .await;
    unregister_embedding(PROVIDER_OPENHUMAN);
    assert!(built.is_ok());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_registered_catalog_answers_the_model_listing_for_its_provider() {
    register_catalog(
        "fake-catalog",
        CatalogEntry {
            list: Arc::new(|_| {
                Box::pin(async { Ok(serde_json::json!({ "models": [{ "id": "registered" }] })) })
            }),
            limits: None,
        },
    );
    let listing =
        crate::inference::provider::models::catalog_listing::list_configured_models_from_config(
            "fake-catalog",
            &Config::default(),
        )
        .await;
    unregister_catalog("fake-catalog");
    assert_eq!(listing.unwrap().value["models"][0]["id"], "registered");
}

#[tokio::test]
async fn without_a_catalog_an_unknown_provider_still_errors() {
    let error =
        crate::inference::provider::models::catalog_listing::list_configured_models_from_config(
            "fake-catalog",
            &Config::default(),
        )
        .await
        .unwrap_err();
    assert!(error.contains("no cloud provider"));
}

#[tokio::test]
async fn a_registered_catalog_supplies_the_managed_limits_request() {
    register_catalog(
        PROVIDER_OPENHUMAN,
        CatalogEntry {
            list: Arc::new(|_| Box::pin(async { Err("unused".to_string()) })),
            limits: Some(Arc::new(|request| {
                Box::pin(async move {
                    Some(DiscoveryRequest::new(
                        "https://catalog.example/v1",
                        &request.model,
                    ))
                })
            })),
        },
    );
    let request =
        model_limits_request("chat", PROVIDER_OPENHUMAN, "some-model", &Config::default()).await;
    unregister_catalog(PROVIDER_OPENHUMAN);
    assert!(request.is_some());
}

/// With nothing registered under the managed provider id, selecting it yields
/// the `BACKEND_UNAVAILABLE:` sentinel instead of a panic or a hosted model.
#[tokio::test]
async fn selecting_the_managed_provider_with_nothing_registered_is_backend_unavailable() {
    let _guard = crate::inference::inference_test_guard();
    let _exclusive = crate::inference::provider::factory::test_provider_override::exclusive();
    let error = make_openhuman_backend_model_for_thread_in(
        None,
        "chat",
        &Config::default(),
        Some("any-model"),
        true,
        None,
        &EgressLedger::none(),
    )
    .err()
    .expect("no managed provider is registered");
    assert!(
        error
            .to_string()
            .starts_with(crate::core::observability::BACKEND_UNAVAILABLE_PREFIX),
        "unexpected error: {error}"
    );
    let one_shot =
        create_chat_model_from_string("chat", PROVIDER_OPENHUMAN, &Config::default(), 0.5).await;
    assert!(one_shot.is_err());
}

/// A configured entry that authenticates with the managed session must hand a
/// registered catalog the key stored for that entry, so a signed-out user with
/// their own key still gets a listing.
#[tokio::test]
async fn a_registered_catalog_receives_the_entrys_provider_key() {
    use crate::config::schema::cloud_providers::{AuthStyle, CloudProviderCreds};
    use crate::config::schema::{EphemeralRoute, EPHEMERAL_ROUTE_SLUG};
    let seen = Arc::new(Mutex::new(String::new()));
    crate::config::schema::cloud_providers::register_hosted_cloud_provider_policy(
        Arc::new(|slug| slug == EPHEMERAL_ROUTE_SLUG),
        Arc::new(|style| style == AuthStyle::OpenhumanJwt),
    );
    let sink = Arc::clone(&seen);
    register_catalog(
        PROVIDER_OPENHUMAN,
        CatalogEntry {
            list: Arc::new(move |request: CatalogRequest<ProviderContext>| {
                *sink.lock().unwrap() = request
                    .context
                    .catalog
                    .as_ref()
                    .unwrap()
                    .request
                    .provider_key
                    .clone();
                Box::pin(async { Ok(serde_json::json!({ "models": [] })) })
            }),
            limits: None,
        },
    );
    let config = Config {
        cloud_providers: vec![CloudProviderCreds {
            id: "p_ephemeral".into(),
            slug: EPHEMERAL_ROUTE_SLUG.into(),
            endpoint: "https://self-hosted.example/v1".into(),
            auth_style: AuthStyle::OpenhumanJwt,
            ..Default::default()
        }],
        ephemeral_route: Some(EphemeralRoute {
            endpoint: "https://self-hosted.example/v1".into(),
            api_key: "entry-key".into(),
            headers: vec![],
            ..EphemeralRoute::default()
        }),
        ..Config::default()
    };
    let listing =
        crate::inference::provider::models::catalog_listing::list_configured_models_from_config(
            "p_ephemeral",
            &config,
        )
        .await;
    unregister_catalog(PROVIDER_OPENHUMAN);
    listing.unwrap();
    assert_eq!(seen.lock().unwrap().as_str(), "entry-key");
}

#[tokio::test]
async fn inspection_model_factory_uses_the_callers_context_before_process_fallback() {
    use crate::core::slots::Extensions;
    use crate::inference::registry::{
        register_inspection_chat_model, register_inspection_chat_model_in,
        unregister_inspection_chat_model, InspectionChatModelBuild,
        PreparedInspectionChatModelFactory,
    };

    let _guard = crate::inference::inference_test_guard_async().await;
    let factory = |endpoint: &'static str| -> PreparedInspectionChatModelFactory {
        Arc::new(move |request| {
            Box::pin(async move {
                Ok(InspectionChatModelBuild {
                    model: Arc::new(FakeModel),
                    model_id: format!(
                        "{endpoint}:{}",
                        request.context.config.default_model.as_deref().unwrap()
                    ),
                    endpoint: format!("https://{endpoint}.example/v1"),
                    credential: format!("{endpoint}-credential"),
                })
            })
        })
    };
    register_inspection_chat_model(PROVIDER_OPENHUMAN, factory("process"));
    register_chat_model(PROVIDER_OPENHUMAN, fake_entry(Arc::default()));
    let a = Extensions::new();
    register_inspection_chat_model_in(&a, PROVIDER_OPENHUMAN, factory("context-a"));
    register_chat_model_in(&a, PROVIDER_OPENHUMAN, fake_entry(Arc::default()));
    let context_a = crate::core::runtime::DispatchCtx::new(
        crate::core::runtime::CoreContext::for_test_with_host_kind_and_extensions(
            crate::core::types::HostKind::Library,
            crate::core::runtime::DomainSet::full(),
            None,
            a,
        ),
    );
    let b = Extensions::new();
    register_inspection_chat_model_in(&b, PROVIDER_OPENHUMAN, factory("context-b"));
    register_chat_model_in(&b, PROVIDER_OPENHUMAN, fake_entry(Arc::default()));
    let context_b = crate::core::runtime::DispatchCtx::new(
        crate::core::runtime::CoreContext::for_test_with_host_kind_and_extensions(
            crate::core::types::HostKind::Library,
            crate::core::runtime::DomainSet::full(),
            None,
            b,
        ),
    );

    let mut config_a = Config::default();
    config_a.default_model = Some("context-a".into());
    let mut config_b = Config::default();
    config_b.default_model = Some("context-b".into());
    let mut config_process = Config::default();
    config_process.default_model = Some("process".into());
    let build_a =
        create_chat_model_with_identity_in_for_inspection(&context_a, "chat", &config_a, 0.0)
            .await
            .expect("context A inspection provider");
    let build_b =
        create_chat_model_with_identity_in_for_inspection(&context_b, "chat", &config_b, 0.0)
            .await
            .expect("context B inspection provider");
    let build_process = create_chat_model_with_identity_in_for_inspection(
        &crate::core::runtime::DispatchCtx::ambient(),
        "chat",
        &config_process,
        0.0,
    )
    .await
    .expect("process inspection provider");
    unregister_inspection_chat_model(PROVIDER_OPENHUMAN);
    unregister_chat_model(PROVIDER_OPENHUMAN);

    assert_eq!(build_a.model_id, "context-a:context-a");
    assert_eq!(build_b.model_id, "context-b:context-b");
    assert_eq!(build_process.model_id, "process:process");
}
