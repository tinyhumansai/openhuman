use super::*;
use tempfile::TempDir;
use tinyinference_llm::providers::overrides::{ChatModelEntry, ProviderModelMetadata};

#[tokio::test]
async fn cached_probe_serves_definitive_negative_results() {
    use crate::core::runtime::{CoreContext, DispatchCtx, DomainSet};
    use crate::core::slots::Extensions;
    use crate::inference::registry::register_chat_model_in;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    // Acquire the async inference guard before the synchronous provider
    // override lock, matching the global lock order used by other fixtures.
    let _guard = crate::inference::inference_test_guard_async().await;
    let _exclusive = crate::inference::provider::factory::test_provider_override::exclusive();
    let tmp = TempDir::new().unwrap();
    let config = Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    let hit_count = std::sync::Arc::new(AtomicUsize::new(0));
    // The core owns no hosted provider; this scoped host registration supplies
    // a deterministic negative probe and counts cache misses without touching
    // a global provider or a real backend.
    let counter = hit_count.clone();
    let extensions = Extensions::new();
    register_chat_model_in(
        &extensions,
        crate::inference::provider::factory::PROVIDER_OPENHUMAN,
        ChatModelEntry {
            build: Arc::new(|_| {
                Ok((
                    Arc::new(tinyagents_harness::testkit::ScriptedModel::replies(
                        ["ready"].to_vec(),
                    )),
                    "fixture-model".to_string(),
                ))
            }),
            probe: Some(Arc::new(move |_| {
                counter.fetch_add(1, Ordering::SeqCst);
                Box::pin(async { Err("API key not configured for provider".to_string()) })
            })),
            models: Some(ProviderModelMetadata {
                default_model: Arc::new(|| Some("fixture-model".to_string())),
                role_for_model: Arc::new(|_| None),
                is_alias: Arc::new(|_| false),
                is_raw_model: Arc::new(|model| !model.trim().is_empty()),
                supports_vision: Arc::new(|_| false),
                resolve_model: Arc::new(|_, _, _| Some("fixture-model".to_string())),
                context_window: Arc::new(|_| None),
                default_pricing: Arc::new(|| None),
                without_reasoning: Arc::new(tinyinference_llm::model::without_reasoning),
            }),
        },
    );
    let core = CoreContext::for_test_with_host_kind_and_extensions(
        crate::core::types::HostKind::Library,
        DomainSet::full(),
        Some(config.workspace_dir.clone()),
        extensions,
    );
    let ctx = DispatchCtx::new(core);

    CoreContext::scope(ctx.resolved().expect("test context"), async {
        let first = cached_probe(&ctx, "summarization", &config).await;
        assert!(first
            .unwrap_err()
            .to_ascii_lowercase()
            .contains("api key not configured for provider"));
        assert_eq!(hit_count.load(Ordering::SeqCst), 1);
        assert!(cached_probe(&ctx, "summarization", &config).await.is_err());
        assert_eq!(hit_count.load(Ordering::SeqCst), 1);
    })
    .await;
}
