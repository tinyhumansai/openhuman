use super::*;
use crate::inference::provider::factory::{
    create_chat_model_from_string, make_openhuman_backend_model_for_thread, model_limits_request,
    PROVIDER_OPENHUMAN,
};
use async_trait::async_trait;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use tinyinference_llm::model::{ModelRequest, ModelResponse};

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

fn fake_entry(seen: Arc<Mutex<Vec<(String, Option<String>, Option<String>)>>>) -> ChatModelEntry {
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
    }
}

#[test]
fn nothing_registered_resolves_no_provider() {
    let _guard = crate::inference::inference_test_guard();
    let _bare = crate::inference::registry_test_support::without_managed_provider();
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

    let built = make_openhuman_backend_model_for_thread(
        "chat",
        &Config::default(),
        "hint:reasoning",
        true,
        Some("thread-1"),
        &EgressLedger::none(),
    );
    let one_shot =
        create_chat_model_from_string("chat", PROVIDER_OPENHUMAN, &Config::default(), 0.5);
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
            Some(crate::config::MODEL_MANAGED_DEFAULT.to_string()),
            Some("thread-1".to_string())
        )
    );
    assert_eq!(
        seen.len(),
        2,
        "both construction paths consulted the registry"
    );
}

#[test]
fn a_registered_embedding_factory_builds_the_managed_embedding_model() {
    let _guard = crate::inference::inference_test_guard();
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&calls);
    register_embedding(PROVIDER_OPENHUMAN, move |request| {
        assert_eq!(request.dimensions, 8);
        counter.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(tinyinference_embeddings::NoopEmbeddingModel))
    });
    let built = crate::inference::embedding_host::create_embedding_provider_with_credentials(
        "cloud",
        "any-model",
        8,
        "",
        None,
    );
    unregister_embedding(PROVIDER_OPENHUMAN);
    assert!(built.is_ok());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_registered_catalog_answers_the_model_listing_for_its_provider() {
    register_catalog(
        "fake-catalog",
        CatalogEntry {
            list: Arc::new(|_, _| {
                Box::pin(async {
                    Ok(Outcome::new(
                        serde_json::json!({ "models": [{ "id": "registered" }] }),
                        vec![],
                    ))
                })
            }),
            limits: None,
        },
    );
    let listing = crate::inference::provider::ops::list_configured_models_from_config(
        "fake-catalog",
        &Config::default(),
    )
    .await;
    unregister_catalog("fake-catalog");
    assert_eq!(listing.unwrap().value["models"][0]["id"], "registered");
}

#[tokio::test]
async fn without_a_catalog_an_unknown_provider_still_errors() {
    let error = crate::inference::provider::ops::list_configured_models_from_config(
        "fake-catalog",
        &Config::default(),
    )
    .await
    .unwrap_err();
    assert!(error.contains("no cloud provider"));
}

#[test]
fn a_registered_catalog_supplies_the_managed_limits_request() {
    register_catalog(
        PROVIDER_OPENHUMAN,
        CatalogEntry {
            list: Arc::new(|_, _| Box::pin(async { Err("unused".to_string()) })),
            limits: Some(Arc::new(|model, _| {
                Some(DiscoveryRequest::new("https://catalog.example/v1", model))
            })),
        },
    );
    let request =
        model_limits_request("chat", PROVIDER_OPENHUMAN, "some-model", &Config::default());
    unregister_catalog(PROVIDER_OPENHUMAN);
    assert!(request.is_some());
}

/// With nothing registered under the managed provider id, selecting it yields
/// the `BACKEND_UNAVAILABLE:` sentinel instead of a panic or a hosted model.
#[test]
fn selecting_the_managed_provider_with_nothing_registered_is_backend_unavailable() {
    let _guard = crate::inference::inference_test_guard();
    let _bare = crate::inference::registry_test_support::without_managed_provider();
    let _exclusive = crate::inference::provider::factory::test_provider_override::exclusive();
    let error = make_openhuman_backend_model_for_thread(
        "chat",
        &Config::default(),
        "any-model",
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
        create_chat_model_from_string("chat", PROVIDER_OPENHUMAN, &Config::default(), 0.5);
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
    let sink = Arc::clone(&seen);
    register_catalog(
        PROVIDER_OPENHUMAN,
        CatalogEntry {
            list: Arc::new(move |_, request| {
                *sink.lock().unwrap() = request.provider_key.clone();
                Box::pin(async { Ok(Outcome::new(serde_json::json!({ "models": [] }), vec![])) })
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
        }),
        ..Config::default()
    };
    let listing =
        crate::inference::provider::ops::list_configured_models_from_config("p_ephemeral", &config)
            .await;
    unregister_catalog(PROVIDER_OPENHUMAN);
    listing.unwrap();
    assert_eq!(seen.lock().unwrap().as_str(), "entry-key");
}
