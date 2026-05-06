//! Local-vs-remote provider resolver for triage turns.
//!
//! ## What this does
//!
//! [`resolve_provider`] always builds the remote provider. Local AI is never
//! used for chat triage — the local path has been removed to guarantee that
//! a triage turn never errors due to Ollama unavailability.
//!
//! `ResolvedProvider.used_local` is preserved for telemetry compatibility but
//! is always `false`.

use std::sync::Arc;

use anyhow::Context;

use crate::openhuman::config::Config;
use crate::openhuman::providers::{self, Provider, ProviderRuntimeOptions, INFERENCE_BACKEND_ID};

/// The concrete provider + metadata that [`crate::openhuman::agent::triage::evaluator::run_triage`]
/// should use for this particular triage turn.
pub struct ResolvedProvider {
    /// Ready-to-use provider, already constructed.
    pub provider: Arc<dyn Provider>,
    /// Provider name token — always `"openhuman"` (remote backend).
    /// Kept for telemetry / observability compat with the previous two-path design.
    pub provider_name: String,
    /// Model identifier — the concrete string `run_tool_call_loop`
    /// will hand to the provider.
    pub model: String,
    /// Always `false` — local AI is never used for triage.
    /// Preserved so existing telemetry subscribers that read this field do not
    /// need code changes.
    pub used_local: bool,
}

// ── Public API ──────────────────────────────────────────────────────────

/// Resolve a provider for a single triage turn. Always returns the remote
/// backend — local AI is hard-disabled for the chat/triage path.
pub async fn resolve_provider() -> anyhow::Result<ResolvedProvider> {
    let config = Config::load_or_init()
        .await
        .context("loading config for triage provider resolution")?;
    resolve_provider_with_config(&config).await
}

/// Inner half of [`resolve_provider`] that takes an already-loaded
/// [`Config`]. Exposed for tests and for the evaluator's retry path.
pub async fn resolve_provider_with_config(config: &Config) -> anyhow::Result<ResolvedProvider> {
    tracing::debug!(
        runtime_enabled = config.local_ai.runtime_enabled,
        "[triage::routing] resolving provider (always remote)"
    );
    build_remote_provider(config)
}

// ── Provider builder ────────────────────────────────────────────────────

/// Build the default remote routed backend provider. Same wiring as
/// `local_ai::ops::agent_chat_simple` uses so we stay consistent with
/// the existing direct-chat path.
fn build_remote_provider(config: &Config) -> anyhow::Result<ResolvedProvider> {
    let default_model = config
        .default_model
        .clone()
        .unwrap_or_else(|| crate::openhuman::config::DEFAULT_MODEL.to_string());
    let options = ProviderRuntimeOptions {
        auth_profile_override: None,
        openhuman_dir: config.config_path.parent().map(std::path::PathBuf::from),
        secrets_encrypt: config.secrets.encrypt,
        reasoning_enabled: config.runtime.reasoning_enabled,
    };
    let provider_box = providers::create_routed_provider_with_options(
        config.api_url.as_deref(),
        config.api_key.as_deref(),
        &config.reliability,
        &config.model_routes,
        default_model.as_str(),
        &options,
    )
    .context("building routed remote provider for triage")?;
    // `Box<dyn Provider>` → `Arc<dyn Provider>` is a single reallocation
    // — the `Provider` trait is `Send + Sync` so this is type-safe.
    let provider: Arc<dyn Provider> = Arc::from(provider_box);
    tracing::debug!(
        provider = %INFERENCE_BACKEND_ID,
        model = %default_model,
        "[triage::routing] resolved remote provider"
    );
    Ok(ResolvedProvider {
        provider,
        provider_name: INFERENCE_BACKEND_ID.to_string(),
        model: default_model,
        used_local: false,
    })
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "routing_tests.rs"]
mod tests;
