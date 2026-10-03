//! Model/provider config operations: AI providers, memory, runtime, local AI, Composio.

use crate::config::Config;
use crate::core::Outcome;

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
    pub model_routes: Option<Vec<crate::config::ModelRouteConfig>>,
    /// When `Some`, REPLACES the entire `config.cloud_providers` array with
    /// the supplied entries (each lacking the API key — those live in
    /// `auth-profiles.json` via [`crate::security::credentials::AuthService`]).
    /// Pass `Some(vec![])` to clear all third-party cloud providers.
    pub cloud_providers: Option<Vec<crate::config::schema::cloud_providers::CloudProviderCreds>>,
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

#[derive(Debug, Clone, Default)]
pub struct ComposioTriggerSettingsPatch {
    /// When `Some(true)`, disables triage for all toolkits.
    pub triage_disabled: Option<bool>,
    /// When `Some(v)`, replaces the per-toolkit opt-out list entirely.
    pub triage_disabled_toolkits: Option<Vec<String>>,
}

/// Which agent-turn roles the incoming patch pinned explicitly.
#[derive(Debug, Clone, Copy, Default)]
struct ExplicitRolePins {
    chat: bool,
    reasoning: bool,
    agentic: bool,
    coding: bool,
}

/// The `cloud_providers` slug [`complete_byok_route`] registers under.
///
/// Distinct from `ephemeral_route::EPHEMERAL_ROUTE_SLUG`: that one is
/// in-memory only and must never be persisted, while this entry exists
/// precisely to be saved.
const BYOK_INFERENCE_SLUG: &str = "byok-inference";

/// Two endpoints are the same route if they differ only by trailing slashes.
fn same_endpoint(a: &str, b: &str) -> bool {
    a.trim().trim_end_matches('/') == b.trim().trim_end_matches('/')
}

/// Make an `inference_url` + `api_key` pair actually route.
///
/// Setting those two is the documented way to point inference at a custom
/// OpenAI-compatible endpoint ("When set together with `api_key`, inference
/// goes direct to this URL instead of the OpenHuman backend"). It did not work:
/// `provider_for_role` resolves through `cloud_providers`, never through
/// `inference_url`, so the save succeeded and the *next turn* died with
///
/// ```text
/// [chat-factory] BYOK_INCOMPLETE: inference_url is set to a custom/direct
/// endpoint (…) but no matching cloud_providers entry was found for role 'chat'
/// ```
///
/// — a failure in a different subsystem, one call later, for a write the API
/// accepted. The caller had to also hand-build the provider entry and pin four
/// roles to `<slug>:<model>`, which is not what the field promises and is not
/// discoverable from the error.
///
/// So complete the statement here, the same way
/// [`ephemeral_route::apply`](crate::config::schema::ephemeral_route::apply)
/// completes it for a single call: register the endpoint as a provider and pin
/// the four roles an agent turn runs on.
///
/// Deliberately conservative:
/// - does nothing unless BOTH `inference_url` and `api_key` are non-blank — an
///   endpoint with no credential is a partial statement, and guessing the other
///   half is how a turn ends up somewhere the caller did not ask for;
/// - does nothing when an entry already matches the endpoint, so a
///   hand-configured provider is never overwritten;
/// - needs a resolved `default_model`, because the provider grammar is
///   `<slug>:<model>` and pinning a role to `<slug>:` trades a working default
///   for a resolution failure;
/// - leaves any role the same patch pinned explicitly, and any role already
///   pinned to something other than the managed default, alone.
fn complete_byok_route(config: &mut Config, explicit: &ExplicitRolePins) {
    use crate::config::schema::cloud_providers::{AuthStyle, CloudProviderCreds};

    let Some(endpoint) = config
        .inference_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
    else {
        return;
    };
    let has_key = config
        .api_key
        .as_deref()
        .map(str::trim)
        .is_some_and(|value| !value.is_empty());
    if !has_key {
        return;
    }

    let existing = config
        .cloud_providers
        .iter()
        .find(|entry| same_endpoint(&entry.endpoint, &endpoint))
        .map(|entry| entry.slug.trim().to_string());

    let Some(model) = config
        .default_model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
    else {
        if existing.is_none() {
            log::warn!(
                "[config][byok] inference_url is set with a key but no default_model is \
                 resolved — cannot complete the BYOK route; turns will report BYOK_INCOMPLETE"
            );
        }
        return;
    };

    let slug = match existing {
        Some(slug) => slug,
        None => {
            log::info!(
                "[config][byok] registering cloud provider '{BYOK_INFERENCE_SLUG}' for the \
                 configured inference_url so agent turns can resolve it"
            );
            config.cloud_providers.push(CloudProviderCreds {
                id: BYOK_INFERENCE_SLUG.to_string(),
                slug: BYOK_INFERENCE_SLUG.to_string(),
                label: "Custom inference endpoint".to_string(),
                endpoint: endpoint.clone(),
                auth_style: AuthStyle::Bearer,
                legacy_type: None,
                default_model: Some(model.clone()),
            });
            BYOK_INFERENCE_SLUG.to_string()
        }
    };

    let provider_string = format!("{slug}:{model}");
    // "Unspoken for" means empty or the managed `cloud` sentinel. Anything else
    // is a deliberate choice — a local Ollama role, a second BYOK provider —
    // and repointing it at this endpoint would be exactly the silent
    // repointing this function exists to avoid.
    let unspoken = |value: &Option<String>| {
        value
            .as_deref()
            .map(str::trim)
            .is_none_or(|v| v.is_empty() || v == "cloud")
    };
    let mut pinned: Vec<&str> = Vec::new();
    for (role, pinned_explicitly, slot) in [
        ("chat", explicit.chat, &mut config.chat_provider),
        (
            "reasoning",
            explicit.reasoning,
            &mut config.reasoning_provider,
        ),
        ("agentic", explicit.agentic, &mut config.agentic_provider),
        ("coding", explicit.coding, &mut config.coding_provider),
    ] {
        if pinned_explicitly || !unspoken(slot) {
            continue;
        }
        *slot = Some(provider_string.clone());
        pinned.push(role);
    }
    if !pinned.is_empty() {
        log::info!(
            "[config][byok] pinned role(s) [{}] to '{}' from the configured inference_url",
            pinned.join(", "),
            provider_string
        );
    }
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
            if crate::inference::provider::factory::is_known_openhuman_tier(m) {
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

    config.save().await.map_err(|e| e.to_string())?;
    let snapshot = snapshot_config_json(config)?;
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
    update: ModelSettingsPatch,
) -> Result<Outcome<serde_json::Value>, String> {
    let mut config = load_config_with_timeout().await?;
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
    config.save().await.map_err(|e| e.to_string())?;
    let snapshot = snapshot_config_json(config)?;
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
    update: MemorySettingsPatch,
) -> Result<Outcome<serde_json::Value>, String> {
    let mut config = load_config_with_timeout().await?;
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
        if effort.is_empty() {
            config.runtime.reasoning_effort = None;
        } else {
            let parsed = crate::agent::tinyagents::parse_reasoning_effort(effort)
                .ok_or_else(|| format!("unknown reasoning_effort '{effort}'"))?;
            config.runtime.reasoning_effort = Some(parsed.as_str().to_string());
        }
    }
    config.save().await.map_err(|e| e.to_string())?;
    let snapshot = snapshot_config_json(config)?;
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
    update: RuntimeSettingsPatch,
) -> Result<Outcome<serde_json::Value>, String> {
    let mut config = load_config_with_timeout().await?;
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
    config.save().await.map_err(|e| e.to_string())?;
    let snapshot = snapshot_config_json(config)?;
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
    update: LocalAiSettingsPatch,
) -> Result<Outcome<serde_json::Value>, String> {
    let mut config = load_config_with_timeout().await?;
    apply_local_ai_settings(&mut config, update).await
}

/// Updates the Composio trigger-triage settings in the configuration.
pub async fn apply_composio_trigger_settings(
    config: &mut Config,
    update: ComposioTriggerSettingsPatch,
) -> Result<Outcome<serde_json::Value>, String> {
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
    Ok(Outcome::new(
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
) -> Result<Outcome<serde_json::Value>, String> {
    let mut config = load_config_with_timeout().await?;
    apply_composio_trigger_settings(&mut config, update).await
}

/// Reads the current composio trigger-triage settings.
pub async fn get_composio_trigger_settings() -> Result<Outcome<serde_json::Value>, String> {
    let config = load_config_with_timeout().await?;
    let result = serde_json::json!({
        "triage_disabled": config.composio.triage_disabled,
        "triage_disabled_toolkits": config.composio.triage_disabled_toolkits,
    });
    Ok(Outcome::new(
        result,
        vec!["composio trigger settings read".to_string()],
    ))
}

/// Resolve the hosted backend URL through the installed backend transport,
/// which excludes local or third-party inference overrides that must never
/// receive OpenHuman session credentials. `None` when no transport is
/// installed (no hosted backend).
pub(crate) fn resolve_backend_api_url(config: &Config) -> Option<String> {
    crate::backend::base_url(&config.api_url).ok()
}

/// Resolves the effective backend API URL from configuration or defaults;
/// `api_url` is `null` when the core has no hosted backend.
pub async fn load_and_resolve_api_url() -> Result<Outcome<serde_json::Value>, String> {
    let config = load_config_with_timeout().await?;
    let resolved = resolve_backend_api_url(&config);
    Ok(Outcome::new(
        serde_json::json!({ "api_url": resolved }),
        Vec::new(),
    ))
}

#[cfg(test)]
#[path = "model_byok_tests.rs"]
mod byok_tests;
