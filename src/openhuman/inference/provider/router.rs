use super::traits::{ChatMessage, ChatRequest, ChatResponse};
use super::Provider;
use async_trait::async_trait;
use std::collections::HashMap;

/// Maps OpenHuman's abstract tier model names (`reasoning-v1`, `chat-v1`,
/// `reasoning-quick-v1`, `agentic-v1`, `coding-v1`, `summarization-v1`,
/// `vision-v1`) to the hint slot in `model_routes`. Returns `None` for any model the
/// router shouldn't rewrite.
fn openhuman_tier_to_hint(model: &str) -> Option<&'static str> {
    match model {
        "reasoning-v1" => Some("reasoning"),
        "chat-v1" => Some("chat"),
        "reasoning-quick-v1" => Some("chat"),
        "agentic-v1" => Some("agentic"),
        "coding-v1" => Some("coding"),
        "summarization-v1" => Some("summarization"),
        "vision-v1" => Some("vision"),
        _ => None,
    }
}

/// A single route: maps a task hint to a provider + model combo.
#[derive(Debug, Clone)]
pub struct Route {
    pub provider_name: String,
    pub model: String,
    /// Known context window for `model` when discoverable (tokens).
    pub context_window: Option<u64>,
}

/// Multi-model router — routes requests to different provider+model combos
/// based on a task hint encoded in the model parameter.
///
/// The model parameter can be:
/// - A regular model name (e.g. "anthropic/claude-sonnet-4") → uses default provider
/// - A hint-prefixed string (e.g. "hint:reasoning") → resolves via route table
///
/// This wraps multiple pre-created providers and selects the right one per request.
pub struct RouterProvider {
    routes: HashMap<String, (usize, String)>, // hint → (provider_index, model)
    providers: Vec<(String, Box<dyn Provider>)>,
    default_index: usize,
    default_model: String,
}

impl RouterProvider {
    /// Create a new router with a default provider and optional routes.
    ///
    /// `providers` is a list of (name, provider) pairs. The first one is the default.
    /// `routes` maps hint names to Route structs containing provider_name and model.
    pub fn new(
        providers: Vec<(String, Box<dyn Provider>)>,
        routes: Vec<(String, Route)>,
        default_model: String,
    ) -> Self {
        // Build provider name → index lookup
        let name_to_index: HashMap<&str, usize> = providers
            .iter()
            .enumerate()
            .map(|(i, (name, _))| (name.as_str(), i))
            .collect();

        // Resolve routes to provider indices
        let resolved_routes: HashMap<String, (usize, String)> = routes
            .into_iter()
            .filter_map(|(hint, route)| {
                let index = name_to_index.get(route.provider_name.as_str()).copied();
                match index {
                    Some(i) => Some((hint, (i, route.model))),
                    None => {
                        tracing::warn!(
                            hint = hint,
                            provider = route.provider_name,
                            "Route references unknown provider, skipping"
                        );
                        None
                    }
                }
            })
            .collect();

        Self {
            routes: resolved_routes,
            providers,
            default_index: 0,
            default_model,
        }
    }

    /// Resolve a model parameter to a (provider, actual_model) pair.
    ///
    /// Resolution order:
    /// 1. `hint:<name>` — direct hint lookup (e.g. `hint:reasoning`).
    /// 2. OpenHuman abstract tier names — `reasoning-v1`, `chat-v1`,
    ///    `agentic-v1`, `coding-v1`, `summarization-v1` map onto the corresponding hints
    ///    so a custom provider gets the user-configured model id instead of
    ///    the literal tier name (which is only meaningful to the OpenHuman
    ///    backend and would 404 on OpenAI/Anthropic/etc.).
    /// 3. Anything else passes through unchanged to the default provider.
    fn resolve(&self, model: &str) -> (usize, String) {
        if let Some(hint) = model.strip_prefix("hint:") {
            if let Some((idx, resolved_model)) = self.routes.get(hint) {
                log::info!(
                    "[router] hint:{} -> model={} (provider_idx={})",
                    hint,
                    resolved_model,
                    idx
                );
                return (*idx, resolved_model.clone());
            }
            tracing::warn!(
                hint = hint,
                "Unknown route hint, falling back to default provider"
            );
        }

        // OpenHuman abstract tier → hint mapping. These names are internal
        // aliases the OpenHuman backend dispatches itself; custom providers
        // need them translated through the user's route table.
        if let Some(hint) = openhuman_tier_to_hint(model) {
            if let Some((idx, resolved_model)) = self.routes.get(hint) {
                log::info!(
                    "[router] tier {} -> hint={} -> model={} (provider_idx={})",
                    model,
                    hint,
                    resolved_model,
                    idx
                );
                return (*idx, resolved_model.clone());
            }
            // Tier name matched but the user hasn't configured a route for
            // this hint. Passing the literal alias (`reasoning-v1`,
            // `agentic-v1`, etc.) verbatim to an upstream API will 400 —
            // these are OpenHuman-internal aliases that only the hosted
            // backend resolves. Fall back to the default provider's
            // default_model so the request at least has a chance of
            // succeeding against a custom_openai / OpenRouter / DeepSeek
            // endpoint. See #2079 (39 events in Sentry from a user routed
            // to DeepSeek who saw "The supported API model names are
            // deepseek-v4-pro or deepseek-v4-flash, but you passed
            // reasoning-v1").
            log::warn!(
                "[router] tier {} matched hint={} but no route configured — falling back to default_model={} on default provider (idx={})",
                model,
                hint,
                self.default_model,
                self.default_index
            );
            return (self.default_index, self.default_model.clone());
        }

        // Not a hint or hint not found — use default provider with the model as-is
        log::info!(
            "[router] passthrough model={} (provider_idx={})",
            model,
            self.default_index
        );
        (self.default_index, model.to_string())
    }
}

#[async_trait]
impl Provider for RouterProvider {
    async fn chat_with_system(
        &self,
        system_prompt: Option<&str>,
        message: &str,
        model: &str,
        temperature: f64,
    ) -> anyhow::Result<String> {
        let (provider_idx, resolved_model) = self.resolve(model);

        let (provider_name, provider) = &self.providers[provider_idx];
        tracing::info!(
            provider = provider_name.as_str(),
            model = resolved_model.as_str(),
            "Router dispatching request"
        );

        provider
            .chat_with_system(system_prompt, message, &resolved_model, temperature)
            .await
    }

    async fn chat_with_history(
        &self,
        messages: &[ChatMessage],
        model: &str,
        temperature: f64,
    ) -> anyhow::Result<String> {
        let (provider_idx, resolved_model) = self.resolve(model);
        let (_, provider) = &self.providers[provider_idx];
        provider
            .chat_with_history(messages, &resolved_model, temperature)
            .await
    }

    async fn chat(
        &self,
        request: ChatRequest<'_>,
        model: &str,
        temperature: f64,
    ) -> anyhow::Result<ChatResponse> {
        let (provider_idx, resolved_model) = self.resolve(model);
        let (_, provider) = &self.providers[provider_idx];
        provider.chat(request, &resolved_model, temperature).await
    }

    async fn chat_with_tools(
        &self,
        messages: &[ChatMessage],
        tools: &[serde_json::Value],
        model: &str,
        temperature: f64,
    ) -> anyhow::Result<ChatResponse> {
        let (provider_idx, resolved_model) = self.resolve(model);
        let (_, provider) = &self.providers[provider_idx];
        provider
            .chat_with_tools(messages, tools, &resolved_model, temperature)
            .await
    }

    fn supports_native_tools(&self) -> bool {
        self.providers
            .get(self.default_index)
            .map(|(_, p)| p.supports_native_tools())
            .unwrap_or(false)
    }

    fn supports_vision(&self) -> bool {
        self.providers
            .iter()
            .any(|(_, provider)| provider.supports_vision())
    }

    /// Delegate to the provider that actually handles `model` so local
    /// runtimes report their runtime-loaded window (LM Studio `n_ctx`) instead
    /// of the static-table default the trait would otherwise return (#3550 /
    /// TAURI-RUST-6V0).
    async fn effective_context_window(&self, model: &str) -> Option<u64> {
        let (provider_idx, resolved_model) = self.resolve(model);
        let (_, provider) = &self.providers[provider_idx];
        provider.effective_context_window(&resolved_model).await
    }

    /// Whether the *default* provider is local. Model-blind — kept for callers
    /// that have no model in hand. The engine's pre-dispatch guard uses the
    /// model-aware [`Provider::is_local_provider_for_model`] below instead, so a
    /// cloud-default router still gates correctly when it routes a model to a
    /// local provider (#3550 / TAURI-RUST-6V0).
    fn is_local_provider(&self) -> bool {
        self.providers
            .get(self.default_index)
            .map(|(_, p)| p.is_local_provider())
            .unwrap_or(false)
    }

    /// Resolve `model` to the provider that actually handles it and report
    /// *that* provider's locality. Without this, a router whose default is
    /// cloud reports `is_local_provider() == false` even when `model` routes to
    /// a local provider, so the engine's pre-dispatch un-evictable-prefix guard
    /// is skipped and the opaque local `400 (n_keep >= n_ctx)` reaches the user
    /// (Codex P2 + CodeRabbit review on PR #3771). `effective_context_window`
    /// already resolves the routed provider, so this keeps the two in step.
    fn is_local_provider_for_model(&self, model: &str) -> bool {
        let (provider_idx, _) = self.resolve(model);
        self.providers
            .get(provider_idx)
            .map(|(_, p)| p.is_local_provider())
            .unwrap_or(false)
    }

    /// Delegate the authoritative runtime-loaded window to the routed provider,
    /// mirroring [`RouterProvider::effective_context_window`] so the engine's
    /// hard pre-dispatch abort sees the same routed provider's loaded `n_ctx`
    /// (#3550 / TAURI-RUST-6V0).
    async fn loaded_context_window(&self, model: &str) -> Option<u64> {
        let (provider_idx, resolved_model) = self.resolve(model);
        let (_, provider) = &self.providers[provider_idx];
        provider.loaded_context_window(&resolved_model).await
    }

    async fn warmup(&self) -> anyhow::Result<()> {
        for (name, provider) in &self.providers {
            tracing::info!(provider = name, "Warming up routed provider");
            if let Err(e) = provider.warmup().await {
                tracing::warn!(provider = name, "Warmup failed (non-fatal): {e}");
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "router_tests.rs"]
mod router_tests;
