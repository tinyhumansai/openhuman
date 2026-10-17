use super::*;
use std::sync::Mutex as StdMutex;

use serde_json::{json, Value};
use tempfile::TempDir;
use tinyinference_llm::model::discover::ModelLimitsCache;

use crate::config::schema::cloud_providers::{AuthStyle, CloudProviderCreds};
use crate::config::schema::ModelRegistryEntry;
use crate::security::credentials::AuthService;

const OPENROUTER: &str = "https://openrouter.ai/api/v1";
const V41_FLASH: &str = "deepseek/deepseek-v4.1-flash";

/// Canned JSON per URL; never touches the network.
#[derive(Default)]
struct FakeFetcher {
    routes: Vec<(String, Value)>,
    posts: Vec<(String, Value)>,
    post_calls: StdMutex<Vec<String>>,
    calls: StdMutex<Vec<String>>,
}

impl FakeFetcher {
    fn with(mut self, url: &str, body: Value) -> Self {
        self.routes.push((url.to_string(), body));
        self
    }

    fn with_post(mut self, url: &str, body: Value) -> Self {
        self.posts.push((url.to_string(), body));
        self
    }

    fn post_count(&self) -> usize {
        self.post_calls.lock().unwrap().len()
    }

    fn call_count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }
}

#[async_trait::async_trait]
impl ModelListingFetcher for FakeFetcher {
    async fn get_json(
        &self,
        url: &str,
        _headers: &[(String, String)],
    ) -> tinyinference_llm::Result<Value> {
        self.calls.lock().unwrap().push(url.to_string());
        self.routes
            .iter()
            .find(|(route, _)| route == url)
            .map(|(_, body)| body.clone())
            .ok_or_else(|| tinyinference_llm::Error::Catalog(format!("GET {url} returned 404")))
    }

    async fn post_json(
        &self,
        url: &str,
        _headers: &[(String, String)],
        _body: &Value,
    ) -> tinyinference_llm::Result<Value> {
        self.post_calls.lock().unwrap().push(url.to_string());
        self.posts
            .iter()
            .find(|(route, _)| route == url)
            .map(|(_, body)| body.clone())
            .ok_or_else(|| tinyinference_llm::Error::Catalog(format!("POST {url} returned 404")))
    }
}

fn openrouter_listing() -> Value {
    json!({ "data": [{
        "id": V41_FLASH,
        "context_length": 1_048_576,
        "top_provider": { "context_length": 1_048_576, "max_completion_tokens": 65_536 }
    }]})
}

fn openrouter_config(tmp: &TempDir) -> Config {
    let mut config = Config::default();
    config.workspace_dir = tmp.path().join("workspace");
    config.config_path = tmp.path().join("config.toml");
    config.cloud_providers = vec![CloudProviderCreds {
        id: "p_openrouter".to_string(),
        slug: "openrouter".to_string(),
        label: "OpenRouter".to_string(),
        endpoint: OPENROUTER.to_string(),
        auth_style: AuthStyle::Bearer,
        ..Default::default()
    }];
    AuthService::from_config(&config)
        .store_provider_token(
            "provider:openrouter",
            "default",
            "sk-or-test",
            Default::default(),
            true,
        )
        .expect("store provider token");
    config
}

async fn resolve(
    fetcher: &FakeFetcher,
    cache: &ModelLimitsCache,
    config: &Config,
    model: &str,
) -> ResolvedWindow {
    resolve_context_window_with(
        fetcher,
        cache,
        "chat",
        &format!("openrouter:{model}"),
        model,
        config,
    )
    .await
}

#[tokio::test]
async fn deepseek_v41_flash_uses_provider_window_not_static_128k() {
    let tmp = TempDir::new().unwrap();
    let config = openrouter_config(&tmp);
    let fetcher =
        FakeFetcher::default().with(&format!("{OPENROUTER}/models"), openrouter_listing());
    let cache = ModelLimitsCache::default();

    // The static tables guess 128k for any `deepseek` id ...
    assert_eq!(
        crate::inference::model_context::static_context_window_for_model(V41_FLASH),
        Some(128_000)
    );
    // ... but the provider says ~1M, and the provider wins.
    let resolved = resolve(&fetcher, &cache, &config, V41_FLASH).await;
    assert_eq!(resolved.window, Some(1_048_576));
    assert_eq!(resolved.source, WindowSource::ProviderReported);

    // The route-aware synchronous lookup used by the context breakdown
    // reports the same provider-specific window.
    assert_eq!(
        crate::inference::model_context::context_window_for_route(
            &format!("openrouter:{V41_FLASH}"),
            V41_FLASH,
            &config,
        ),
        Some(1_048_576)
    );
}

#[tokio::test]
async fn second_resolution_is_served_from_cache() {
    let tmp = TempDir::new().unwrap();
    let config = openrouter_config(&tmp);
    let fetcher =
        FakeFetcher::default().with(&format!("{OPENROUTER}/models"), openrouter_listing());
    let cache = ModelLimitsCache::default();
    resolve(&fetcher, &cache, &config, V41_FLASH).await;
    resolve(&fetcher, &cache, &config, V41_FLASH).await;
    assert_eq!(fetcher.call_count(), 1);
}

#[tokio::test]
async fn config_override_wins_over_provider() {
    let tmp = TempDir::new().unwrap();
    let mut config = openrouter_config(&tmp);
    let model = "deepseek/override-test-model";
    config.model_registry.push(ModelRegistryEntry {
        id: model.to_string(),
        provider: "openrouter".to_string(),
        context_window: 64_000,
        ..Default::default()
    });
    let fetcher = FakeFetcher::default().with(
        &format!("{OPENROUTER}/models"),
        json!({ "data": [{ "id": model, "context_length": 1_048_576 }] }),
    );
    let resolved = resolve(&fetcher, &ModelLimitsCache::default(), &config, model).await;
    assert_eq!(resolved.window, Some(64_000));
    assert_eq!(resolved.source, WindowSource::ConfigOverride);
    assert_eq!(fetcher.call_count(), 0, "an override skips discovery");
    assert_eq!(remembered_window("openrouter", model), None);
    assert_eq!(
        crate::inference::model_context::context_window_for_route(
            &format!("openrouter:{model}"),
            model,
            &config,
        ),
        Some(64_000)
    );
}

#[tokio::test]
async fn learned_overflow_limit_corrects_the_listing() {
    let tmp = TempDir::new().unwrap();
    let config = openrouter_config(&tmp);
    let model = "deepseek/learned-test-model";
    let fetcher = FakeFetcher::default().with(
        &format!("{OPENROUTER}/models"),
        json!({ "data": [{ "id": model, "context_length": 1_048_576 }] }),
    );
    let cache = ModelLimitsCache::default();
    tinyinference_llm::model::discover::record_overflow_error_in(
        &cache,
        OPENROUTER,
        model,
        "This endpoint's maximum context length is 163840 tokens. However, you requested about 300000 tokens",
    );
    let resolved = resolve(&fetcher, &cache, &config, model).await;
    assert_eq!(resolved.window, Some(163_840));
    assert_eq!(resolved.source, WindowSource::LearnedFromOverflow);
}

#[tokio::test]
async fn unreachable_provider_falls_back_to_static_guess() {
    let tmp = TempDir::new().unwrap();
    let config = openrouter_config(&tmp);
    let fetcher = FakeFetcher::default();
    let model = "deepseek/unlisted-test-model";
    let resolved = resolve(&fetcher, &ModelLimitsCache::default(), &config, model).await;
    assert_eq!(resolved.window, Some(128_000));
    assert_eq!(resolved.source, WindowSource::StaticGuess);
}

#[tokio::test]
async fn unknown_model_without_provider_data_is_unknown() {
    let tmp = TempDir::new().unwrap();
    let config = openrouter_config(&tmp);
    let resolved = resolve(
        &FakeFetcher::default(),
        &ModelLimitsCache::default(),
        &config,
        "vendor/totally-unknown-xyz",
    )
    .await;
    assert_eq!(
        resolved,
        ResolvedWindow {
            window: None,
            source: WindowSource::Unknown,
            max_output_tokens: None,
        }
    );
}

#[tokio::test]
async fn local_ollama_without_a_reachable_server_falls_back_to_the_local_profile() {
    let tmp = TempDir::new().unwrap();
    let config = openrouter_config(&tmp);
    let fetcher = FakeFetcher::default();
    let resolved = resolve_context_window_with(
        &fetcher,
        &ModelLimitsCache::default(),
        "chat",
        "ollama:some-local-model",
        "some-local-model",
        &config,
    )
    .await;
    assert!(resolved.window.is_some());
    assert_eq!(resolved.source, WindowSource::LocalProfile);
}

fn ollama_show() -> Value {
    json!({
        "model_info": { "general.architecture": "qwen3", "qwen3.context_length": 40960 },
        "capabilities": ["completion", "tools"]
    })
}

#[tokio::test]
async fn builtin_local_ollama_reads_the_window_from_api_show() {
    let tmp = TempDir::new().unwrap();
    let mut config = openrouter_config(&tmp);
    config.local_ai.base_url = Some("http://127.0.0.1:11434".to_string());
    let fetcher =
        FakeFetcher::default().with_post("http://127.0.0.1:11434/api/show", ollama_show());
    let resolved = resolve_context_window_with(
        &fetcher,
        &ModelLimitsCache::default(),
        "chat",
        "ollama:qwen3:14b",
        "qwen3:14b",
        &config,
    )
    .await;
    assert_eq!(resolved.window, Some(40_960));
    assert_eq!(resolved.source, WindowSource::ProviderReported);
}

#[tokio::test]
async fn custom_openai_provider_at_ollama_v1_reads_the_window_from_api_show() {
    let tmp = TempDir::new().unwrap();
    let mut config = openrouter_config(&tmp);
    config.cloud_providers.push(CloudProviderCreds {
        id: "p_ollama".to_string(),
        slug: "myollama".to_string(),
        label: "Ollama".to_string(),
        endpoint: "http://127.0.0.1:11434/v1".to_string(),
        auth_style: AuthStyle::None,
        ..Default::default()
    });
    // `/v1/models` lists the model without any window, as Ollama does.
    let fetcher = FakeFetcher::default()
        .with(
            "http://127.0.0.1:11434/v1/models",
            json!({"data": [{"id": "qwen3:14b", "object": "model", "owned_by": "library"}]}),
        )
        .with_post("http://127.0.0.1:11434/api/show", ollama_show());
    let cache = ModelLimitsCache::default();
    let resolve = || {
        resolve_context_window_with(
            &fetcher,
            &cache,
            "chat",
            "myollama:qwen3:14b",
            "qwen3:14b",
            &config,
        )
    };
    let resolved = resolve().await;
    assert_eq!(resolved.window, Some(40_960));
    assert_eq!(resolved.source, WindowSource::ProviderReported);
    // Cached: the second turn makes no further requests.
    resolve().await;
    assert_eq!(fetcher.post_count(), 1);
}

#[tokio::test]
async fn failed_ollama_probe_is_remembered_and_not_retried() {
    let tmp = TempDir::new().unwrap();
    let mut config = openrouter_config(&tmp);
    config.local_ai.base_url = Some("http://127.0.0.1:11434".to_string());
    // The resolver remembers provider-sourced windows process-wide, so this
    // test uses a model id no other test resolves successfully.
    let fetcher = FakeFetcher::default();
    let cache = ModelLimitsCache::default();
    for _ in 0..3 {
        let resolved = resolve_context_window_with(
            &fetcher,
            &cache,
            "chat",
            "ollama:qwen3:failed-probe",
            "qwen3:failed-probe",
            &config,
        )
        .await;
        assert_eq!(resolved.source, WindowSource::LocalProfile);
    }
    assert_eq!(fetcher.post_count(), 1);
}

#[test]
fn config_override_ignores_zero_and_other_models() {
    let mut config = Config::default();
    config.model_registry.push(ModelRegistryEntry {
        id: "a".to_string(),
        context_window: 0,
        ..Default::default()
    });
    config.model_registry.push(ModelRegistryEntry {
        id: "b".to_string(),
        context_window: 32_000,
        ..Default::default()
    });
    assert_eq!(config_override("a", &config), None);
    assert_eq!(config_override(" b ", &config), Some(32_000));
    assert_eq!(config_override("c", &config), None);
}
