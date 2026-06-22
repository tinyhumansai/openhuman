use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

struct MockProvider {
    calls: Arc<AtomicUsize>,
    response: &'static str,
    last_model: parking_lot::Mutex<String>,
    /// When set, this mock reports as a local runtime and exposes an
    /// authoritative loaded context window — used to exercise the model-aware
    /// locality routing for the engine's pre-dispatch prefix guard (#3771).
    local_loaded_ctx: Option<u64>,
}

impl MockProvider {
    fn new(response: &'static str) -> Self {
        Self {
            calls: Arc::new(AtomicUsize::new(0)),
            response,
            last_model: parking_lot::Mutex::new(String::new()),
            local_loaded_ctx: None,
        }
    }

    /// A mock that reports as local with the given authoritative loaded window.
    fn new_local(response: &'static str, loaded_ctx: u64) -> Self {
        Self {
            calls: Arc::new(AtomicUsize::new(0)),
            response,
            last_model: parking_lot::Mutex::new(String::new()),
            local_loaded_ctx: Some(loaded_ctx),
        }
    }

    fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn last_model(&self) -> String {
        self.last_model.lock().clone()
    }
}

#[async_trait]
impl Provider for MockProvider {
    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        model: &str,
        _temperature: f64,
    ) -> anyhow::Result<String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        *self.last_model.lock() = model.to_string();
        Ok(self.response.to_string())
    }

    fn is_local_provider(&self) -> bool {
        self.local_loaded_ctx.is_some()
    }

    async fn loaded_context_window(&self, _model: &str) -> Option<u64> {
        self.local_loaded_ctx
    }
}

fn make_router(
    providers: Vec<(&'static str, &'static str)>,
    routes: Vec<(&str, &str, &str)>,
) -> (RouterProvider, Vec<Arc<MockProvider>>) {
    let mocks: Vec<Arc<MockProvider>> = providers
        .iter()
        .map(|(_, response)| Arc::new(MockProvider::new(response)))
        .collect();

    let provider_list: Vec<(String, Box<dyn Provider>)> = providers
        .iter()
        .zip(mocks.iter())
        .map(|((name, _), mock)| {
            (
                name.to_string(),
                Box::new(Arc::clone(mock)) as Box<dyn Provider>,
            )
        })
        .collect();

    let route_list: Vec<(String, Route)> = routes
        .iter()
        .map(|(hint, provider_name, model)| {
            (
                hint.to_string(),
                Route {
                    provider_name: provider_name.to_string(),
                    model: model.to_string(),
                    context_window: None,
                },
            )
        })
        .collect();

    let router = RouterProvider::new(provider_list, route_list, "default-model".to_string());

    (router, mocks)
}

#[async_trait]
impl Provider for Arc<MockProvider> {
    async fn chat_with_system(
        &self,
        system_prompt: Option<&str>,
        message: &str,
        model: &str,
        temperature: f64,
    ) -> anyhow::Result<String> {
        self.as_ref()
            .chat_with_system(system_prompt, message, model, temperature)
            .await
    }

    fn is_local_provider(&self) -> bool {
        self.as_ref().is_local_provider()
    }

    async fn loaded_context_window(&self, model: &str) -> Option<u64> {
        self.as_ref().loaded_context_window(model).await
    }
}

#[tokio::test]
async fn routes_hint_to_correct_provider() {
    let (router, mocks) = make_router(
        vec![("fast", "fast-response"), ("smart", "smart-response")],
        vec![
            ("fast", "fast", "llama-3-70b"),
            ("reasoning", "smart", "claude-opus"),
        ],
    );

    let result = router
        .simple_chat("hello", "hint:reasoning", 0.5)
        .await
        .unwrap();
    assert_eq!(result, "smart-response");
    assert_eq!(mocks[1].call_count(), 1);
    assert_eq!(mocks[1].last_model(), "claude-opus");
    assert_eq!(mocks[0].call_count(), 0);
}

#[tokio::test]
async fn routes_fast_hint() {
    let (router, mocks) = make_router(
        vec![("fast", "fast-response"), ("smart", "smart-response")],
        vec![("fast", "fast", "llama-3-70b")],
    );

    let result = router.simple_chat("hello", "hint:fast", 0.5).await.unwrap();
    assert_eq!(result, "fast-response");
    assert_eq!(mocks[0].call_count(), 1);
    assert_eq!(mocks[0].last_model(), "llama-3-70b");
}

#[tokio::test]
async fn unknown_hint_falls_back_to_default() {
    let (router, mocks) = make_router(
        vec![("default", "default-response"), ("other", "other-response")],
        vec![],
    );

    let result = router
        .simple_chat("hello", "hint:nonexistent", 0.5)
        .await
        .unwrap();
    assert_eq!(result, "default-response");
    assert_eq!(mocks[0].call_count(), 1);
    assert_eq!(mocks[0].last_model(), "hint:nonexistent");
}

#[tokio::test]
async fn non_hint_model_uses_default_provider() {
    let (router, mocks) = make_router(
        vec![
            ("primary", "primary-response"),
            ("secondary", "secondary-response"),
        ],
        vec![("code", "secondary", "codellama")],
    );

    let result = router
        .simple_chat("hello", "anthropic/claude-sonnet-4-20250514", 0.5)
        .await
        .unwrap();
    assert_eq!(result, "primary-response");
    assert_eq!(mocks[0].call_count(), 1);
    assert_eq!(mocks[0].last_model(), "anthropic/claude-sonnet-4-20250514");
}

#[test]
fn resolve_preserves_model_for_non_hints() {
    let (router, _) = make_router(vec![("default", "ok")], vec![]);

    let (idx, model) = router.resolve("gpt-4o");
    assert_eq!(idx, 0);
    assert_eq!(model, "gpt-4o");
}

#[test]
fn resolve_strips_hint_prefix() {
    let (router, _) = make_router(
        vec![("fast", "ok"), ("smart", "ok")],
        vec![("reasoning", "smart", "claude-opus")],
    );

    let (idx, model) = router.resolve("hint:reasoning");
    assert_eq!(idx, 1);
    assert_eq!(model, "claude-opus");
}

#[test]
fn resolve_translates_openhuman_tier_aliases_via_route_table() {
    let (router, _) = make_router(
        vec![("default", "ok"), ("smart", "ok")],
        vec![
            ("reasoning", "smart", "gpt-5.5"),
            ("chat", "smart", "gpt-5.5-mini"),
            ("summarization", "smart", "gpt-4.1-nano"),
            ("vision", "smart", "gpt-5.5-vision"),
        ],
    );

    let (reasoning_idx, reasoning_model) = router.resolve("reasoning-v1");
    assert_eq!(reasoning_idx, 1);
    assert_eq!(reasoning_model, "gpt-5.5");

    let (chat_idx, chat_model) = router.resolve("chat-v1");
    assert_eq!(chat_idx, 1);
    assert_eq!(chat_model, "gpt-5.5-mini");

    let (summary_idx, summary_model) = router.resolve("summarization-v1");
    assert_eq!(summary_idx, 1);
    assert_eq!(summary_model, "gpt-4.1-nano");

    // The vision tier alias routes through the `vision` hint to the BYOK model.
    let (vision_idx, vision_model) = router.resolve("vision-v1");
    assert_eq!(vision_idx, 1);
    assert_eq!(vision_model, "gpt-5.5-vision");
}

// -- #2079: tier alias must not leak to upstream when no route configured ---

#[test]
fn tier_alias_falls_back_to_default_model_when_no_route_is_configured() {
    // Regression for #2079. A user with a custom_openai provider pointed at
    // DeepSeek (default_model = "deepseek-v4-pro") and no explicit route
    // for the `reasoning` hint used to see the literal alias
    // "reasoning-v1" forwarded to the upstream API, which DeepSeek rejects
    // with: "The supported API model names are deepseek-v4-pro or
    // deepseek-v4-flash, but you passed reasoning-v1."
    //
    // After the fix, the router falls back to the default provider's
    // default_model so the request has a chance of succeeding.
    let mocks: Vec<Arc<MockProvider>> = (0..1).map(|_| Arc::new(MockProvider::new("ok"))).collect();
    let provider_list: Vec<(String, Box<dyn Provider>)> = vec![(
        "deepseek".to_string(),
        Box::new(Arc::clone(&mocks[0])) as Box<dyn Provider>,
    )];
    let router = RouterProvider::new(provider_list, vec![], "deepseek-v4-pro".to_string());

    let (idx, model) = router.resolve("reasoning-v1");
    assert_eq!(idx, 0, "fall back to default provider index");
    assert_eq!(
        model, "deepseek-v4-pro",
        "fall back to default_model, NOT the literal tier alias"
    );
}

#[test]
fn every_tier_alias_falls_back_to_default_model_when_unrouted() {
    // Exhaustive check across the alias set in openhuman_tier_to_hint —
    // confirms no tier name slips through and gets forwarded verbatim.
    let mocks: Vec<Arc<MockProvider>> = (0..1).map(|_| Arc::new(MockProvider::new("ok"))).collect();
    let provider_list: Vec<(String, Box<dyn Provider>)> = vec![(
        "custom".to_string(),
        Box::new(Arc::clone(&mocks[0])) as Box<dyn Provider>,
    )];
    let router = RouterProvider::new(provider_list, vec![], "user-configured-model".to_string());

    for alias in [
        "reasoning-v1",
        "chat-v1",
        "reasoning-quick-v1",
        "agentic-v1",
        "coding-v1",
        "summarization-v1",
        "vision-v1",
    ] {
        let (idx, model) = router.resolve(alias);
        assert_eq!(idx, 0, "alias {} → default provider index", alias);
        assert_eq!(
            model, "user-configured-model",
            "alias {} must NOT leak verbatim to the upstream API; expected default_model fallback",
            alias
        );
    }
}

#[test]
fn passthrough_for_unknown_model_name_still_sends_string_verbatim() {
    // Regression guard for the existing pass-through branch. A model name
    // the router doesn't recognise (e.g. an upstream-native model id like
    // "deepseek-v4-flash" or "claude-opus-4.5") must still be forwarded
    // verbatim — the fallback we added in the previous test must only fire
    // for the listed tier aliases, never as a generic catch-all.
    let mocks: Vec<Arc<MockProvider>> = (0..1).map(|_| Arc::new(MockProvider::new("ok"))).collect();
    let provider_list: Vec<(String, Box<dyn Provider>)> = vec![(
        "custom".to_string(),
        Box::new(Arc::clone(&mocks[0])) as Box<dyn Provider>,
    )];
    let router = RouterProvider::new(provider_list, vec![], "default-model".to_string());

    let (idx, model) = router.resolve("deepseek-v4-flash");
    assert_eq!(idx, 0);
    assert_eq!(
        model, "deepseek-v4-flash",
        "non-alias model names must continue to pass through unchanged"
    );

    let (idx2, model2) = router.resolve("anthropic/claude-opus-4.5");
    assert_eq!(idx2, 0);
    assert_eq!(model2, "anthropic/claude-opus-4.5");
}

#[test]
fn skips_routes_with_unknown_provider() {
    let (router, _) = make_router(
        vec![("default", "ok")],
        vec![("broken", "nonexistent", "model")],
    );

    assert!(!router.routes.contains_key("broken"));
}

// -- #3771: model-aware locality for the pre-dispatch un-evictable-prefix guard --

#[tokio::test]
async fn is_local_provider_for_model_resolves_routed_provider_not_default() {
    // Default provider is CLOUD; a hint routes a model to a LOCAL provider.
    // The model-blind `is_local_provider()` reports the default (cloud), but
    // the model-aware `is_local_provider_for_model()` must follow the route to
    // the local provider so the engine arms its prefix guard (Codex P2 +
    // CodeRabbit PR #3771). `effective_context_window`/`loaded_context_window`
    // already resolve the route, so all three must agree per-model.
    let cloud = Arc::new(MockProvider::new("cloud"));
    let local = Arc::new(MockProvider::new_local("local", 8_192));
    let router = RouterProvider::new(
        vec![
            (
                "cloud".to_string(),
                Box::new(Arc::clone(&cloud)) as Box<dyn Provider>,
            ),
            (
                "local".to_string(),
                Box::new(Arc::clone(&local)) as Box<dyn Provider>,
            ),
        ],
        vec![(
            "fast".to_string(),
            Route {
                provider_name: "local".to_string(),
                model: "qwen3:8b".to_string(),
                context_window: None,
            },
        )],
        "cloud-default-model".to_string(),
    );

    // Model-blind locality reflects the (cloud) default.
    assert!(
        !router.is_local_provider(),
        "default provider is cloud → model-blind locality is false"
    );

    // A model that routes to the local provider must report local + expose its
    // authoritative loaded window.
    assert!(
        router.is_local_provider_for_model("hint:fast"),
        "a model routed to the local provider must report local"
    );
    assert_eq!(
        router.loaded_context_window("hint:fast").await,
        Some(8_192),
        "loaded window must come from the routed local provider"
    );

    // A model that falls through to the cloud default must NOT report local and
    // has no authoritative loaded window.
    assert!(
        !router.is_local_provider_for_model("anthropic/claude-sonnet-4"),
        "a model on the cloud default must not report local"
    );
    assert_eq!(
        router
            .loaded_context_window("anthropic/claude-sonnet-4")
            .await,
        None,
        "cloud provider exposes no authoritative loaded window"
    );
}

#[tokio::test]
async fn warmup_calls_all_providers() {
    let (router, _) = make_router(vec![("a", "ok"), ("b", "ok")], vec![]);

    assert!(router.warmup().await.is_ok());
}

#[tokio::test]
async fn chat_with_system_passes_system_prompt() {
    let mock = Arc::new(MockProvider::new("response"));
    let router = RouterProvider::new(
        vec![(
            "default".into(),
            Box::new(Arc::clone(&mock)) as Box<dyn Provider>,
        )],
        vec![],
        "model".into(),
    );

    let result = router
        .chat_with_system(Some("system"), "hello", "model", 0.5)
        .await
        .unwrap();
    assert_eq!(result, "response");
    assert_eq!(mock.call_count(), 1);
}

#[tokio::test]
async fn chat_with_tools_delegates_to_resolved_provider() {
    let mock = Arc::new(MockProvider::new("tool-response"));
    let router = RouterProvider::new(
        vec![(
            "default".into(),
            Box::new(Arc::clone(&mock)) as Box<dyn Provider>,
        )],
        vec![],
        "model".into(),
    );

    let messages = vec![ChatMessage {
        id: None,
        role: "user".to_string(),
        content: "use tools".to_string(),
        extra_metadata: None,
    }];
    let tools = vec![serde_json::json!({
        "type": "function",
        "function": {
            "name": "shell",
            "description": "Run shell command",
            "parameters": {}
        }
    })];

    let result = router
        .chat_with_tools(&messages, &tools, "model", 0.7)
        .await
        .unwrap();
    assert_eq!(result.text.as_deref(), Some("tool-response"));
    assert_eq!(mock.call_count(), 1);
    assert_eq!(mock.last_model(), "model");
}

#[tokio::test]
async fn chat_with_tools_routes_hint_correctly() {
    let (router, mocks) = make_router(
        vec![("fast", "fast-tool"), ("smart", "smart-tool")],
        vec![("reasoning", "smart", "claude-opus")],
    );

    let messages = vec![ChatMessage {
        id: None,
        role: "user".to_string(),
        content: "reason about this".to_string(),
        extra_metadata: None,
    }];
    let tools = vec![serde_json::json!({"type": "function", "function": {"name": "test"}})];

    let result = router
        .chat_with_tools(&messages, &tools, "hint:reasoning", 0.5)
        .await
        .unwrap();
    assert_eq!(result.text.as_deref(), Some("smart-tool"));
    assert_eq!(mocks[1].call_count(), 1);
    assert_eq!(mocks[1].last_model(), "claude-opus");
    assert_eq!(mocks[0].call_count(), 0);
}
