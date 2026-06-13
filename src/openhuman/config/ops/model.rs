//! Model/provider config operations: AI providers, memory, runtime, local AI, Composio.

use crate::openhuman::config::Config;
use crate::rpc::RpcOutcome;

use super::loader::{load_config_with_timeout, snapshot_config_json};

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
    pub model_routes: Option<Vec<crate::openhuman::config::ModelRouteConfig>>,
    /// When `Some`, REPLACES the entire `config.cloud_providers` array with
    /// the supplied entries (each lacking the API key — those live in
    /// `auth-profiles.json` via [`crate::openhuman::credentials::AuthService`]).
    /// Pass `Some(vec![])` to clear all third-party cloud providers.
    pub cloud_providers:
        Option<Vec<crate::openhuman::config::schema::cloud_providers::CloudProviderCreds>>,
    /// When `Some`, REPLACES the entire `config.model_registry` array. Carries
    /// each model's user-set `vision` flag (Settings → Advanced LLM → custom
    /// model → "Supports vision"). Pass `Some(vec![])` to clear; `None` keeps it.
    pub model_registry: Option<Vec<crate::openhuman::config::schema::ModelRegistryEntry>>,
    /// Id of the `cloud_providers` entry used when a workload routes to
    /// `"cloud"`. Empty string clears (factory falls back to OpenHuman).
    pub primary_cloud: Option<String>,
    pub chat_provider: Option<String>,
    pub reasoning_provider: Option<String>,
    pub agentic_provider: Option<String>,
    pub coding_provider: Option<String>,
    pub memory_provider: Option<String>,
    pub embeddings_provider: Option<String>,
    pub heartbeat_provider: Option<String>,
    pub learning_provider: Option<String>,
    pub subconscious_provider: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct MemorySettingsPatch {
    pub backend: Option<String>,
    pub auto_save: Option<bool>,
    pub embedding_provider: Option<String>,
    pub embedding_model: Option<String>,
    pub embedding_dimensions: Option<usize>,
    /// Stepped user-facing memory-context window preset (see
    /// [`crate::openhuman::config::schema::agent::MemoryContextWindow`]).
    /// Accepts `"minimal" | "balanced" | "extended" | "maximum"`.
    /// Unknown values are silently ignored so old clients can keep
    /// posting partial patches.
    pub memory_window: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct RuntimeSettingsPatch {
    pub kind: Option<String>,
    pub reasoning_enabled: Option<bool>,
}

#[derive(Debug, Clone, Default)]
pub struct LocalAiSettingsPatch {
    pub runtime_enabled: Option<bool>,
    /// MVP opt-in marker. Bootstrap hard-overrides status to "disabled"
    /// when this is `false`, regardless of `runtime_enabled`. The unified
    /// AI panel ties the two together (both flip on enable, both flip
    /// off on disable) so a single toggle gives the user the obvious
    /// behaviour without needing to apply a preset first.
    pub opt_in_confirmed: Option<bool>,
    pub provider: Option<String>,
    pub base_url: Option<Option<String>>,
    pub model_id: Option<String>,
    pub chat_model_id: Option<String>,
    pub usage_embeddings: Option<bool>,
    pub usage_heartbeat: Option<bool>,
    pub usage_learning_reflection: Option<bool>,
    pub usage_subconscious: Option<bool>,
}

#[derive(Debug, Clone, Default)]
pub struct ComposioTriggerSettingsPatch {
    /// When `Some(true)`, disables triage for all toolkits.
    pub triage_disabled: Option<bool>,
    /// When `Some(v)`, replaces the per-toolkit opt-out list entirely.
    pub triage_disabled_toolkits: Option<Vec<String>>,
}

/// Updates the model-related settings in the configuration.
pub async fn apply_model_settings(
    config: &mut Config,
    update: ModelSettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
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
            if !crate::openhuman::inference::provider::factory::is_known_openhuman_tier(m) {
                log::warn!(
                    "[config][model-settings] default_model '{}' is not a recognized \
                     OpenHuman backend tier — it will be replaced with the platform \
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
        use crate::openhuman::config::schema::cloud_providers::is_slug_reserved;
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
    }
    if let Some(primary) = update.primary_cloud {
        let trimmed = primary.trim();
        config.primary_cloud = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        };
    }

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
    if let Some(s) = update.memory_provider {
        config.memory_provider = normalise_provider(s);
    }
    if let Some(s) = update.embeddings_provider {
        config.embeddings_provider = normalise_provider(s);
    }
    if let Some(s) = update.heartbeat_provider {
        config.heartbeat_provider = normalise_provider(s);
    }
    if let Some(s) = update.learning_provider {
        config.learning_provider = normalise_provider(s);
    }
    if let Some(s) = update.subconscious_provider {
        config.subconscious_provider = normalise_provider(s);
    }

    config.save().await.map_err(|e| e.to_string())?;
    // #1574 §4: the AIPanel workload matrix changes the embedder via THIS
    // (model-settings) path — `embeddings_provider` above — not the
    // memory-settings path. Trigger the same idempotent re-embed backfill
    // so a UI embedder switch recovers prior memory under the new
    // signature. Coverage-gated + non-fatal: if the active signature did
    // not actually change, this enqueues nothing.
    crate::openhuman::memory_queue::ensure_reembed_backfill(config);
    let snapshot = snapshot_config_json(config)?;
    Ok(RpcOutcome::new(
        snapshot,
        vec![format!(
            "model settings saved to {}",
            config.config_path.display()
        )],
    ))
}

/// Loads the configuration, applies model settings updates, and saves it.
pub async fn load_and_apply_model_settings(
    update: ModelSettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let mut config = load_config_with_timeout().await?;
    apply_model_settings(&mut config, update).await
}

/// Updates the memory-related settings in the configuration.
pub async fn apply_memory_settings(
    config: &mut Config,
    update: MemorySettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    if let Some(backend) = update.backend {
        config.memory.backend = backend;
    }
    if let Some(auto_save) = update.auto_save {
        config.memory.auto_save = auto_save;
    }
    if let Some(provider) = update.embedding_provider {
        config.memory.embedding_provider = provider;
    }
    if let Some(model) = update.embedding_model {
        config.memory.embedding_model = model;
    }
    if let Some(dimensions) = update.embedding_dimensions {
        config.memory.embedding_dimensions = dimensions;
    }
    if let Some(window_label) = update.memory_window.as_deref() {
        if let Some(window) =
            crate::openhuman::config::schema::MemoryContextWindow::from_str_opt(window_label)
        {
            config.agent.memory_window = Some(window);
        } else {
            tracing::warn!(
                requested = window_label,
                "[config] unknown memory_window preset — leaving existing setting unchanged"
            );
        }
    }
    config.save().await.map_err(|e| e.to_string())?;
    // #1574 §4: the embedder may have just changed (provider/model/dims).
    // Ensure a re-embed backfill chain exists for the new active signature
    // so prior memory becomes retrievable again instead of silently going
    // dark. Idempotent + non-fatal (covered space enqueues nothing; errors
    // are logged, never fail the settings save). §7's migration is
    // one-shot so it does not cover a later switch — this does.
    crate::openhuman::memory_queue::ensure_reembed_backfill(config);
    let snapshot = snapshot_config_json(config)?;
    Ok(RpcOutcome::new(
        snapshot,
        vec![format!(
            "memory settings saved to {}",
            config.config_path.display()
        )],
    ))
}

/// Loads the configuration, applies memory settings updates, and saves it.
pub async fn load_and_apply_memory_settings(
    update: MemorySettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let mut config = load_config_with_timeout().await?;
    apply_memory_settings(&mut config, update).await
}

/// Updates the runtime-related settings in the configuration.
pub async fn apply_runtime_settings(
    config: &mut Config,
    update: RuntimeSettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    if let Some(kind) = update.kind {
        config.runtime.kind = kind;
    }
    if let Some(reasoning_enabled) = update.reasoning_enabled {
        config.runtime.reasoning_enabled = Some(reasoning_enabled);
    }
    config.save().await.map_err(|e| e.to_string())?;
    let snapshot = snapshot_config_json(config)?;
    Ok(RpcOutcome::new(
        snapshot,
        vec![format!(
            "runtime settings saved to {}",
            config.config_path.display()
        )],
    ))
}

/// Loads the configuration, applies runtime settings updates, and saves it.
pub async fn load_and_apply_runtime_settings(
    update: RuntimeSettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let mut config = load_config_with_timeout().await?;
    apply_runtime_settings(&mut config, update).await
}

/// Updates the local-AI runtime + per-feature usage flags in the configuration.
pub async fn apply_local_ai_settings(
    config: &mut Config,
    update: LocalAiSettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    if let Some(v) = update.runtime_enabled {
        config.local_ai.runtime_enabled = v;
    }
    if let Some(v) = update.opt_in_confirmed {
        config.local_ai.opt_in_confirmed = v;
    }
    if let Some(provider) = update.provider {
        config.local_ai.provider =
            crate::openhuman::inference::local::provider::normalize_provider(&provider);
    }
    if let Some(base_url) = update.base_url {
        config.local_ai.base_url = match base_url {
            None => None,
            Some(base_url) if base_url.trim().is_empty() => None,
            Some(base_url)
                if crate::openhuman::inference::local::provider::provider_from_config(config)
                    == crate::openhuman::inference::local::provider::LocalAiProvider::Ollama =>
            {
                Some(crate::openhuman::inference::local::validate_ollama_url(
                    &base_url,
                )?)
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
    if let Some(v) = update.usage_heartbeat {
        config.local_ai.usage.heartbeat = v;
    }
    if let Some(v) = update.usage_learning_reflection {
        config.local_ai.usage.learning_reflection = v;
    }
    if let Some(v) = update.usage_subconscious {
        config.local_ai.usage.subconscious = v;
    }
    config.save().await.map_err(|e| e.to_string())?;
    let snapshot = snapshot_config_json(config)?;
    Ok(RpcOutcome::new(
        snapshot,
        vec![format!(
            "local AI settings saved to {}",
            config.config_path.display()
        )],
    ))
}

/// Loads the configuration, applies local-AI settings updates, and saves it.
pub async fn load_and_apply_local_ai_settings(
    update: LocalAiSettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let mut config = load_config_with_timeout().await?;
    apply_local_ai_settings(&mut config, update).await
}

/// Updates the Composio trigger-triage settings in the configuration.
pub async fn apply_composio_trigger_settings(
    config: &mut Config,
    update: ComposioTriggerSettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    if let Some(v) = update.triage_disabled {
        config.composio.triage_disabled = v;
        tracing::debug!(
            triage_disabled = v,
            "[config][composio] triage_disabled updated"
        );
    }
    if let Some(toolkits) = update.triage_disabled_toolkits {
        tracing::debug!(
            count = toolkits.len(),
            "[config][composio] triage_disabled_toolkits updated"
        );
        config.composio.triage_disabled_toolkits = toolkits;
    }
    config.save().await.map_err(|e| e.to_string())?;
    let snapshot = snapshot_config_json(config)?;
    Ok(RpcOutcome::new(
        snapshot,
        vec![format!(
            "composio trigger settings saved to {}",
            config.config_path.display()
        )],
    ))
}

/// Loads the configuration, applies composio trigger settings, and saves it.
pub async fn load_and_apply_composio_trigger_settings(
    update: ComposioTriggerSettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let mut config = load_config_with_timeout().await?;
    apply_composio_trigger_settings(&mut config, update).await
}

/// Reads the current composio trigger-triage settings.
pub async fn get_composio_trigger_settings() -> Result<RpcOutcome<serde_json::Value>, String> {
    let config = load_config_with_timeout().await?;
    let result = serde_json::json!({
        "triage_disabled": config.composio.triage_disabled,
        "triage_disabled_toolkits": config.composio.triage_disabled_toolkits,
    });
    Ok(RpcOutcome::new(
        result,
        vec!["composio trigger settings read".to_string()],
    ))
}

/// Resolves the effective API URL from configuration or defaults.
pub async fn load_and_resolve_api_url() -> Result<RpcOutcome<serde_json::Value>, String> {
    let config = load_config_with_timeout().await?;
    let resolved = crate::api::config::effective_api_url(&config.api_url);
    Ok(RpcOutcome::new(
        serde_json::json!({ "api_url": resolved }),
        Vec::new(),
    ))
}
