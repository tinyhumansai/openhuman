//! Model/provider config operations: AI providers, memory, runtime, local AI, Composio.

use crate::config::Config;
use crate::core::runtime::DispatchCtx;
use crate::core::Outcome;

use super::loader::snapshot_config_json;

#[path = "model_byok_route.rs"]
mod model_byok_route;
#[cfg(test)]
use model_byok_route::BYOK_INFERENCE_SLUG;
use model_byok_route::{complete_byok_route, ExplicitRolePins};

#[derive(Debug, Clone, Default)]
pub struct ModelSettingsPatch {
    pub api_url: Option<String>,
    /// Custom OpenAI-compatible LLM endpoint. Empty string clears the
    /// override (inference falls back through the OpenHuman backend).
    pub inference_url: Option<String>,
    pub api_key: Option<String>,
    pub default_model: Option<String>,
    pub default_temperature: Option<f64>,
    /// When `Some`, REPLACES the entire `config.model_routes` array with the
    /// supplied (hint, model) pairs. Pass `Some(vec![])` to clear all routes
    /// (e.g. when switching back to the OpenHuman backend whose built-in
    /// router picks per-task models on its own). Leave `None` to keep the
    /// current routes untouched.
    pub model_routes: Option<Vec<crate::config::ModelRouteConfig>>,
    /// When `Some`, REPLACES the entire `config.cloud_providers` array with
    /// the supplied entries (each lacking the API key — those live in
    /// `auth-profiles.json` via [`crate::credentials::AuthService`]).
    /// Pass `Some(vec![])` to clear all third-party cloud providers.
    pub cloud_providers: Option<Vec<crate::config::schema::cloud_providers::CloudProviderCreds>>,
    /// PEM CA bundle changes keyed by provider slug; empty values remove one.
    pub cloud_provider_ca_certs: Option<std::collections::HashMap<String, String>>,
    /// When `Some`, REPLACES the entire `config.model_registry` array. Carries
    /// each model's user-set `vision` flag (Settings → Advanced LLM → custom
    /// model → "Supports vision"). Pass `Some(vec![])` to clear; `None` keeps it.
    pub model_registry: Option<Vec<crate::config::schema::ModelRegistryEntry>>,
    /// Id of the `cloud_providers` entry used when a workload routes to
    /// `"cloud"`. Empty string clears (factory falls back to OpenHuman).
    pub primary_cloud: Option<String>,
    pub chat_provider: Option<String>,
    pub reasoning_provider: Option<String>,
    pub agentic_provider: Option<String>,
    pub coding_provider: Option<String>,
    pub vision_provider: Option<String>,
    pub memory_provider: Option<String>,
    pub embeddings_provider: Option<String>,
}

/// Collect and validate provider CA bundles from either settings RPC surface.
/// Slugs are normalized here so the map keys match normalized provider entries.
pub fn collect_provider_ca_certs<'a>(
    providers: impl IntoIterator<Item = (&'a str, Option<&'a str>)>,
) -> Result<std::collections::HashMap<String, String>, String> {
    let mut certs = std::collections::HashMap::new();
    for (slug, pem) in providers {
        let Some(pem) = pem else { continue };
        if !pem.is_empty() {
            crate::util::tls::parse_ca_bundle(pem)?;
        }
        certs.insert(slug.trim().to_string(), pem.to_string());
    }
    Ok(certs)
}

#[derive(Debug, Clone, Default)]
pub struct MemorySettingsPatch {
    pub embedding_provider: Option<String>,
    pub embedding_model: Option<String>,
    pub embedding_dimensions: Option<usize>,
}

#[derive(Debug, Clone, Default)]
pub struct RuntimeSettingsPatch {
    pub kind: Option<String>,
    pub reasoning_enabled: Option<bool>,
    /// `Some("")` clears the effort back to the provider default.
    pub reasoning_effort: Option<String>,
    /// When set, `reasoning_effort` is saved as this model's own level
    /// (`runtime.reasoning_effort_by_model`) instead of the global one, and
    /// `Some("")` removes the model's entry.
    pub reasoning_effort_model: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct LocalAiSettingsPatch {
    pub runtime_enabled: Option<bool>,
    /// MVP opt-in marker. Bootstrap hard-overrides status to "disabled"
    /// when this is `false`, regardless of `runtime_enabled`. The unified
    /// AI panel ties the two together (both flip on enable, both flip
    /// off on disable) so a single toggle gives the user the obvious
    /// behaviour.
    pub opt_in_confirmed: Option<bool>,
    pub provider: Option<String>,
    pub base_url: Option<Option<String>>,
    pub model_id: Option<String>,
    pub chat_model_id: Option<String>,
    pub usage_embeddings: Option<bool>,
    pub api_key: Option<String>,
}

/// Updates the model-related settings in the configuration.
pub async fn apply_model_settings(
    config: &mut Config,
    update: ModelSettingsPatch,
) -> Result<Outcome<serde_json::Value>, String> {
    if let Some(api_url) = update.api_url {
        config.api_url = if api_url.trim().is_empty() {
            None
        } else {
            Some(api_url)
        };
    }
    if let Some(inference_url) = update.inference_url {
        config.inference_url = if inference_url.trim().is_empty() {
            None
        } else {
            Some(inference_url.trim().to_string())
        };
    }
    if let Some(api_key) = update.api_key {
        let trimmed_key = api_key.trim();
        config.api_key = if trimmed_key.is_empty() {
            None
        } else {
            Some(trimmed_key.to_string())
        };
    }
    if let Some(model) = update.default_model {
        let trimmed = model.trim();
        config.default_model = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        };
        if let Some(ref m) = config.default_model {
            if crate::inference::registry::provider_model_is_alias(
                crate::inference::provider::factory::PROVIDER_OPENHUMAN,
                m,
            ) {
                log::warn!(
                    "[config][model-settings] default_model '{}' is a retired tier slug or \
                     role hint, not a catalog model — managed turns run on the platform \
                     default at inference time.",
                    m
                );
            }
        }
    }
    if let Some(temp) = update.default_temperature {
        config.default_temperature = temp;
    }
    if let Some(routes) = update.model_routes {
        config.model_routes = routes;
    }
    if let Some(registry) = update.model_registry {
        // Full replacement — the UI sends the canonical per-model registry
        // (carrying each model's `vision` flag). Empty vec clears it.
        log::debug!(
            "[config] apply_model_settings: replacing model_registry ({} entries)",
            registry.len()
        );
        // Normalize ids: `model_vision_enabled` matches the resolved model id
        // exactly, so stray surrounding whitespace would silently disable vision
        // for an otherwise valid model.
        config.model_registry = registry
            .into_iter()
            .map(|mut entry| {
                entry.id = entry.id.trim().to_string();
                entry
            })
            .collect();
    }
    if let Some(providers) = update.cloud_providers {
        use crate::config::schema::cloud_providers::is_slug_reserved;
        let preserved: Vec<_> = config
            .cloud_providers
            .iter()
            .filter(|e| is_slug_reserved(e.slug.trim()))
            .cloned()
            .collect();
        log::debug!(
            "[config] apply_model_settings: preserving {} reserved cloud provider(s) before overwrite",
            preserved.len()
        );
        config.cloud_providers = providers;
        let before_reinject = config.cloud_providers.len();
        for entry in preserved {
            let preserved_slug = entry.slug.trim();
            if !config
                .cloud_providers
                .iter()
                .any(|e| e.slug.trim() == preserved_slug)
            {
                config.cloud_providers.push(entry);
            }
        }
        log::debug!(
            "[config] apply_model_settings: reinjected {} reserved cloud provider(s)",
            config.cloud_providers.len() - before_reinject
        );
        // Filter only after restoring built-ins so their configured CA roots
        // survive a full custom-provider-list replacement.
        config.cloud_provider_ca_certs.retain(|slug, _| {
            config
                .cloud_providers
                .iter()
                .any(|entry| entry.slug.trim() == slug)
        });
    }
    if let Some(certs) = update.cloud_provider_ca_certs {
        for (slug, pem) in certs {
            if pem.is_empty() {
                config.cloud_provider_ca_certs.remove(&slug);
            } else if config
                .cloud_providers
                .iter()
                .any(|entry| entry.slug.trim() == slug)
            {
                config.cloud_provider_ca_certs.insert(slug, pem);
            }
        }
    }
    if let Some(primary) = update.primary_cloud {
        let trimmed = primary.trim();
        config.primary_cloud = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        };
    }

    // Which of the four agent-turn roles this patch pinned itself. A caller
    // that named a role means it; `complete_byok_route` below only fills the
    // ones nobody spoke for.
    let explicit_role_pins = ExplicitRolePins {
        chat: update.chat_provider.is_some(),
        reasoning: update.reasoning_provider.is_some(),
        agentic: update.agentic_provider.is_some(),
        coding: update.coding_provider.is_some(),
    };

    let normalise_provider = |s: String| -> Option<String> {
        let t = s.trim();
        if t.is_empty() {
            None
        } else {
            Some(t.to_string())
        }
    };
    if let Some(s) = update.chat_provider {
        config.chat_provider = normalise_provider(s);
    }
    if let Some(s) = update.reasoning_provider {
        config.reasoning_provider = normalise_provider(s);
    }
    if let Some(s) = update.agentic_provider {
        config.agentic_provider = normalise_provider(s);
    }
    if let Some(s) = update.coding_provider {
        config.coding_provider = normalise_provider(s);
    }
    if let Some(s) = update.vision_provider {
        config.vision_provider = normalise_provider(s);
    }
    if let Some(s) = update.memory_provider {
        config.memory_provider = normalise_provider(s);
    }
    if let Some(s) = update.embeddings_provider {
        config.embeddings_provider = normalise_provider(s);
    }

    complete_byok_route(config, &explicit_role_pins);

    config.save().await.map_err(super::config_save_error)?;
    let snapshot = snapshot_config_json(config).await?;
    Ok(Outcome::new(
        snapshot,
        vec![format!(
            "model settings saved to {}",
            config.config_path.display()
        )],
    ))
}

/// Loads the configuration, applies model settings updates, and saves it.
pub async fn load_and_apply_model_settings(
    ctx: &DispatchCtx,
    update: ModelSettingsPatch,
) -> Result<Outcome<serde_json::Value>, String> {
    let mut config = ctx.load_config().await?;
    apply_model_settings(&mut config, update).await
}

/// Updates the embedding settings kept under `[memory]` and the agent's
/// memory-context window.
pub async fn apply_memory_settings(
    config: &mut Config,
    update: MemorySettingsPatch,
) -> Result<Outcome<serde_json::Value>, String> {
    if let Some(provider) = update.embedding_provider {
        config.memory.embedding_provider = provider;
    }
    if let Some(model) = update.embedding_model {
        // Source-gate (TAURI-RUST-9SK): reject an unmistakably non-embedding
        // model id before persisting it. This save path has no live verify
        // probe (unlike the Custom-provider setup flow in
        // `embeddings::rpc::update_settings`), so a chat model id pasted here
        // would otherwise be stored unchecked and 400 "does not exist" on every
        // memory re-embed (2205 events from one user). Conservative check — see
        // `tinyinference_embeddings::catalog::non_embedding_model_reason`.
        if let Some(reason) = tinyinference_embeddings::catalog::non_embedding_model_reason(&model)
        {
            return Err(format!("invalid embeddings model `{model}`: {reason}"));
        }
        config.memory.embedding_model = model;
    }
    if let Some(dimensions) = update.embedding_dimensions {
        config.memory.embedding_dimensions = dimensions;
    }
    config.save().await.map_err(super::config_save_error)?;
    let snapshot = snapshot_config_json(config).await?;
    Ok(Outcome::new(
        snapshot,
        vec![format!(
            "memory settings saved to {}",
            config.config_path.display()
        )],
    ))
}

/// Loads the configuration, applies memory settings updates, and saves it.
pub async fn load_and_apply_memory_settings(
    ctx: &DispatchCtx,
    update: MemorySettingsPatch,
) -> Result<Outcome<serde_json::Value>, String> {
    let mut config = ctx.load_config().await?;
    apply_memory_settings(&mut config, update).await
}

/// Updates the runtime-related settings in the configuration.
pub async fn apply_runtime_settings(
    config: &mut Config,
    update: RuntimeSettingsPatch,
) -> Result<Outcome<serde_json::Value>, String> {
    if let Some(kind) = update.kind {
        config.runtime.kind = kind;
    }
    if let Some(reasoning_enabled) = update.reasoning_enabled {
        config.runtime.reasoning_enabled = Some(reasoning_enabled);
    }
    if let Some(effort) = update.reasoning_effort {
        let effort = effort.trim();
        let parsed = if effort.is_empty() {
            None
        } else {
            let parsed = tinyinference_llm::model::ReasoningEffort::parse(effort)
                .ok_or_else(|| format!("unknown reasoning_effort '{effort}'"))?;
            Some(parsed.as_str().to_string())
        };
        let model = update.reasoning_effort_model.as_deref().map(str::trim);
        if model == Some("") {
            return Err("reasoning_effort_model must not be empty".into());
        }
        match (model, parsed) {
            (Some(model), Some(effort)) => {
                log::debug!("[config][reasoning] model={model} effort={effort}");
                config
                    .runtime
                    .reasoning_effort_by_model
                    .insert(model.to_string(), effort);
            }
            (Some(model), None) => {
                log::debug!("[config][reasoning] model={model} effort cleared");
                config.runtime.reasoning_effort_by_model.remove(model);
            }
            (None, effort) => config.runtime.reasoning_effort = effort,
        }
    }
    config.save().await.map_err(super::config_save_error)?;
    let snapshot = snapshot_config_json(config).await?;
    Ok(Outcome::new(
        snapshot,
        vec![format!(
            "runtime settings saved to {}",
            config.config_path.display()
        )],
    ))
}

/// Loads the configuration, applies runtime settings updates, and saves it.
pub async fn load_and_apply_runtime_settings(
    ctx: &DispatchCtx,
    update: RuntimeSettingsPatch,
) -> Result<Outcome<serde_json::Value>, String> {
    let mut config = ctx.load_config().await?;
    apply_runtime_settings(&mut config, update).await
}

/// Updates the local-AI runtime + per-feature usage flags in the configuration.
pub async fn apply_local_ai_settings(
    config: &mut Config,
    update: LocalAiSettingsPatch,
) -> Result<Outcome<serde_json::Value>, String> {
    if let Some(v) = update.runtime_enabled {
        config.local_ai.runtime_enabled = v;
    }
    if let Some(v) = update.opt_in_confirmed {
        config.local_ai.opt_in_confirmed = v;
    }
    if let Some(provider) = update.provider {
        config.local_ai.provider = tinyinference_local::provider::normalize_provider(&provider);
    }
    if let Some(base_url) = update.base_url {
        config.local_ai.base_url = match base_url {
            None => None,
            Some(base_url) if base_url.trim().is_empty() => None,
            // OMLX is an OpenAI-v1 endpoint: the `/v1` suffix is significant, so it
            // must NOT go through `validate_ollama_url` (which strips the path).
            // `provider_from_name` maps omlx → Ollama, so guard on the slug here.
            Some(base_url)
                if tinyinference_local::provider::normalize_provider(&config.local_ai.provider)
                    != "omlx"
                    && tinyinference_local::provider::provider_from_name(
                        &config.local_ai.provider,
                    ) == tinyinference_local::provider::LocalAiProvider::Ollama =>
            {
                Some(tinyinference_local::ollama::validate_ollama_url(&base_url)?)
            }
            Some(base_url) => Some(base_url.trim().trim_end_matches('/').to_string()),
        };
    }
    if let Some(model_id) = update.model_id {
        config.local_ai.model_id = model_id.trim().to_string();
    }
    if let Some(chat_model_id) = update.chat_model_id {
        config.local_ai.chat_model_id = chat_model_id.trim().to_string();
    }
    if let Some(v) = update.usage_embeddings {
        config.local_ai.usage.embeddings = v;
    }
    if let Some(api_key) = update.api_key {
        let trimmed = api_key.trim();
        config.local_ai.api_key = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        };
        log::debug!(
            "[config][local_ai] api_key {}",
            if config.local_ai.api_key.is_some() {
                "set"
            } else {
                "cleared"
            }
        );
    }
    config.save().await.map_err(super::config_save_error)?;
    let snapshot = snapshot_config_json(config).await?;
    Ok(Outcome::new(
        snapshot,
        vec![format!(
            "local AI settings saved to {}",
            config.config_path.display()
        )],
    ))
}

/// Loads the configuration, applies local-AI settings updates, and saves it.
pub async fn load_and_apply_local_ai_settings(
    ctx: &DispatchCtx,
    update: LocalAiSettingsPatch,
) -> Result<Outcome<serde_json::Value>, String> {
    let mut config = ctx.load_config().await?;
    apply_local_ai_settings(&mut config, update).await
}

/// The control-plane URL the host's config enricher resolved onto
/// `config.resolved_endpoints`. The enricher excludes local or third-party
/// inference overrides that must never receive OpenHuman session
/// credentials. `None` when no enricher is registered (no hosted backend).
pub(crate) fn resolve_backend_api_url(config: &Config) -> Option<String> {
    config.resolved_endpoints.control_plane_url.clone()
}

/// Resolves the effective backend API URL from configuration or defaults;
/// `api_url` is `null` when the core has no hosted backend.
pub async fn load_and_resolve_api_url(
    ctx: &DispatchCtx,
) -> Result<Outcome<serde_json::Value>, String> {
    let config = ctx.load_config().await?;
    let resolved = resolve_backend_api_url(&config);
    Ok(Outcome::new(
        serde_json::json!({ "api_url": resolved }),
        Vec::new(),
    ))
}

#[cfg(test)]
#[path = "model_byok_tests.rs"]
mod byok_tests;
