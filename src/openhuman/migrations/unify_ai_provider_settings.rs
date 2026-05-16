//! Migration 1 → 2: unify the scattered AI provider settings into the new
//! per-workload provider-string fields, and seed the `cloud_providers` list.
//!
//! ## What this migration consolidates
//!
//! Pre-unification config carried five different vocabularies for the same
//! question — *"where does this LLM workload run?"*:
//!
//! - `inference_url` + `model_routes`        — global cloud preset
//! - `reasoning_provider` / `agentic_provider` / `coding_provider`
//!                                          — per-role chat (#1710, partial)
//! - `local_ai.usage.{embeddings,heartbeat,learning_reflection,subconscious}`
//!                                          — local-vs-cloud booleans
//! - `memory_tree.llm_backend` (+ `cloud_llm_model`) — memory summariser
//!
//! After this migration there is one grammar — provider strings parsed by
//! [`crate::openhuman::providers::factory`] — addressing all eight workloads
//! uniformly:
//!
//! ```text
//! reasoning_provider, agentic_provider, coding_provider,
//! memory_provider,    embeddings_provider, heartbeat_provider,
//! learning_provider,  subconscious_provider
//! ```
//!
//! plus `cloud_providers: Vec<CloudProviderCreds>` and `primary_cloud` for
//! the credential side.
//!
//! ## Behaviour
//!
//! - Pure in-memory mutation of `Config`. The caller (`migrations::run_pending`)
//!   persists the result via `Config::save()` and bumps `schema_version`.
//! - Idempotent: gated on `*.is_none()` per field. A re-run after a previous
//!   successful run is a no-op.
//! - Never touches keys / secrets. API keys remain in
//!   `auth-profiles.json` via [`crate::openhuman::credentials::AuthService`].
//! - Always seeds an `Openhuman` entry into `cloud_providers` (idempotent —
//!   only when the list is empty).
//! - Migrates `inference_url` into a `Custom` cloud provider entry when the
//!   URL doesn't look like the OpenHuman backend.

use crate::openhuman::config::schema::cloud_providers::{
    generate_provider_id, CloudProviderCreds, CloudProviderType,
};
use crate::openhuman::config::Config;

/// Counters returned by [`run`] for diagnostics. Logged at INFO once per
/// successful migration run.
#[derive(Debug, Default, Clone)]
pub struct MigrationStats {
    pub cloud_providers_seeded: usize,
    pub primary_cloud_set: bool,
    pub workload_fields_filled: usize,
}

/// Run the AI-provider unification migration on the given `Config`.
///
/// Synchronous because the body is pure config mutation — no I/O. The caller
/// is responsible for persisting via `Config::save()` once the runner has
/// also bumped `schema_version`.
pub fn run(config: &mut Config) -> anyhow::Result<MigrationStats> {
    let mut stats = MigrationStats::default();

    seed_cloud_providers(config, &mut stats);
    set_primary_cloud(config, &mut stats);
    derive_workload_providers(config, &mut stats);

    log::info!(
        "[migrations][unify-ai] done seeded_providers={} primary_set={} workload_fields_filled={}",
        stats.cloud_providers_seeded,
        stats.primary_cloud_set,
        stats.workload_fields_filled,
    );
    Ok(stats)
}

/// Seed `cloud_providers` with an OpenHuman entry (and optionally a Custom
/// entry derived from a legacy `inference_url`).
fn seed_cloud_providers(config: &mut Config, stats: &mut MigrationStats) {
    if !config.cloud_providers.is_empty() {
        log::debug!(
            "[migrations][unify-ai] cloud_providers already populated ({} entries), skipping seed",
            config.cloud_providers.len()
        );
        return;
    }

    // Always seed the OpenHuman entry — even if api_url is None, the factory
    // resolves a sensible default at runtime.
    let oh_endpoint = config
        .api_url
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| CloudProviderType::Openhuman.default_endpoint().to_string());
    let oh_default_model = config
        .default_model
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "reasoning-v1".to_string());
    config.cloud_providers.push(CloudProviderCreds {
        id: generate_provider_id(&CloudProviderType::Openhuman),
        r#type: CloudProviderType::Openhuman,
        endpoint: oh_endpoint,
        default_model: oh_default_model,
    });
    stats.cloud_providers_seeded += 1;

    // If there's a legacy `inference_url` pointing at a non-OpenHuman
    // endpoint, surface it as a Custom entry so the user keeps their
    // configuration. The actual key continues to live in auth-profiles.json
    // (or in `api_key` on Config — which is OpenHuman's session JWT and
    // doesn't apply here; users will re-enter via the new UI).
    if let Some(raw) = config.inference_url.as_deref() {
        let trimmed = raw.trim();
        if !trimmed.is_empty() && !looks_like_openhuman(trimmed) {
            // Derive a sensible default model from the legacy model_routes
            // (prefer "reasoning" hint, fall back to whatever is set).
            let default_model = config
                .model_routes
                .iter()
                .find(|r| r.hint.eq_ignore_ascii_case("reasoning"))
                .or_else(|| config.model_routes.first())
                .map(|r| r.model.clone())
                .unwrap_or_default();
            config.cloud_providers.push(CloudProviderCreds {
                id: generate_provider_id(&CloudProviderType::Custom),
                r#type: CloudProviderType::Custom,
                endpoint: trimmed.to_string(),
                default_model,
            });
            stats.cloud_providers_seeded += 1;
            log::info!(
                "[migrations][unify-ai] seeded Custom cloud_providers entry from legacy \
                 inference_url_present=true"
            );
        }
    }
}

/// Default `primary_cloud` to the OpenHuman entry (the first one we just
/// seeded, by construction). Idempotent — only sets if currently `None`.
fn set_primary_cloud(config: &mut Config, stats: &mut MigrationStats) {
    if config.primary_cloud.is_some() {
        return;
    }
    let oh = config
        .cloud_providers
        .iter()
        .find(|e| e.r#type == CloudProviderType::Openhuman);
    if let Some(entry) = oh {
        config.primary_cloud = Some(entry.id.clone());
        stats.primary_cloud_set = true;
        log::debug!(
            "[migrations][unify-ai] primary_cloud set to openhuman entry id={}",
            entry.id
        );
    }
}

/// Derive each per-workload `*_provider` field from the legacy flags.
///
/// All fields are gated on `is_none()` — a partially-migrated config skips
/// fields that were already set by a previous run or a hand-edit.
fn derive_workload_providers(config: &mut Config, stats: &mut MigrationStats) {
    let runtime_on = config.local_ai.runtime_enabled;
    let chat_model = config.local_ai.chat_model_id.clone();
    let embed_model = config.local_ai.embedding_model_id.clone();

    let set_field = |field: &mut Option<String>, value: String, stats: &mut MigrationStats| {
        if field.is_none() {
            *field = Some(value);
            stats.workload_fields_filled += 1;
        }
    };

    // Memory summariser — `memory_tree.llm_backend` is `LlmBackend::Cloud | Local`.
    let memory_value = match config.memory_tree.llm_backend {
        crate::openhuman::config::schema::LlmBackend::Local
            if runtime_on && !chat_model.is_empty() =>
        {
            format!("ollama:{}", chat_model)
        }
        _ => "cloud".to_string(),
    };
    set_field(&mut config.memory_provider, memory_value, stats);

    // Embeddings — uses the embedding_model_id, not chat_model_id.
    let embeddings_value =
        if config.local_ai.usage.embeddings && runtime_on && !embed_model.is_empty() {
            format!("ollama:{}", embed_model)
        } else {
            "cloud".to_string()
        };
    set_field(&mut config.embeddings_provider, embeddings_value, stats);

    // The remaining three use the chat model when local.
    let heartbeat_value = if config.local_ai.usage.heartbeat && runtime_on && !chat_model.is_empty()
    {
        format!("ollama:{}", chat_model)
    } else {
        "cloud".to_string()
    };
    set_field(&mut config.heartbeat_provider, heartbeat_value, stats);

    let learning_value =
        if config.local_ai.usage.learning_reflection && runtime_on && !chat_model.is_empty() {
            format!("ollama:{}", chat_model)
        } else {
            "cloud".to_string()
        };
    set_field(&mut config.learning_provider, learning_value, stats);

    let subconscious_value =
        if config.local_ai.usage.subconscious && runtime_on && !chat_model.is_empty() {
            format!("ollama:{}", chat_model)
        } else {
            "cloud".to_string()
        };
    set_field(&mut config.subconscious_provider, subconscious_value, stats);

    // The three chat workloads (reasoning/agentic/coding) intentionally
    // stay None — the factory treats unset as "cloud" which routes to
    // primary_cloud. No equivalent of "force this to local" existed in the
    // legacy config for chat, so there's nothing to derive.
}

/// Heuristic: does the URL look like a configured OpenHuman backend?
///
/// Used to decide whether a non-empty `inference_url` should be migrated
/// into a Custom cloud provider entry. The default OpenHuman backend lives
/// at api.openhuman.ai; staging and dev URLs use the same host pattern.
///
/// Matches only on the host component to avoid false positives from custom
/// endpoints that happen to contain "openhuman" in a path or query string.
fn looks_like_openhuman(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    // Strip scheme if present.
    let without_scheme = lower.split("://").nth(1).unwrap_or(&lower);
    // Strip userinfo and take only the host[:port] part before any path.
    let authority = without_scheme.split('/').next().unwrap_or("");
    let host = authority.split('@').last().unwrap_or(authority);
    let host_no_port = host.split(':').next().unwrap_or(host);
    host_no_port == "api.openhuman.ai"
        || host_no_port.ends_with(".openhuman.ai")
        // Allow bare "openhuman" for local/dev names (e.g. Docker compose service names).
        || host_no_port == "openhuman"
}

#[cfg(test)]
#[path = "unify_ai_provider_settings_tests.rs"]
mod tests;
