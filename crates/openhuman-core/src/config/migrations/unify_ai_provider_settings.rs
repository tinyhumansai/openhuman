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
//! - `local_ai.usage.{embeddings,heartbeat,subconscious}`
//!                                          — local-vs-cloud booleans
//!
//! After this migration there is one grammar — provider strings parsed by
//! [`crate::inference::provider::factory`] — addressing every workload
//! uniformly:
//!
//! ```text
//! reasoning_provider, agentic_provider, coding_provider,
//! memory_provider,    embeddings_provider
//! ```
//!
//! The legacy `local_ai.usage.heartbeat` / `local_ai.usage.subconscious`
//! booleans are not carried forward: the background loops they routed were
//! removed, so there is no workload left to derive.
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
//!   `auth-profiles.json` via [`crate::security::credentials::AuthService`].
//! - Always seeds an `Openhuman` entry into `cloud_providers` (idempotent —
//!   only when the list is empty).
//! - Migrates `inference_url` into a `Custom` cloud provider entry when the
//!   URL doesn't look like the OpenHuman backend.

use crate::config::schema::cloud_providers::{
    builtin_cloud_provider, generate_provider_id, AuthStyle, CloudProviderCreds,
};
use crate::config::Config;

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
pub(super) fn seed_cloud_providers(config: &mut Config, stats: &mut MigrationStats) {
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
        .unwrap_or_else(|| {
            builtin_cloud_provider("openhuman")
                .expect("managed builtin")
                .endpoint
                .to_string()
        });
    let oh_default_model = config
        .default_model
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| crate::config::MODEL_MANAGED_DEFAULT.to_string());
    config.cloud_providers.push(CloudProviderCreds {
        id: generate_provider_id("openhuman"),
        slug: "openhuman".to_string(),
        label: "OpenHuman".to_string(),
        endpoint: oh_endpoint,
        auth_style: AuthStyle::OpenhumanJwt,
        default_model: Some(oh_default_model),
    });
    stats.cloud_providers_seeded += 1;

    let hosts = crate::config::enrichers::managed_hosts_for(config);

    // If there's a legacy `inference_url` pointing at a non-OpenHuman
    // endpoint, surface it as a Custom entry so the user keeps their
    // configuration. The actual key continues to live in auth-profiles.json
    // (or in `api_key` on Config — which is OpenHuman's session JWT and
    // doesn't apply here; users will re-enter via the new UI).
    if let Some(raw) = config.inference_url.as_deref() {
        let trimmed = raw.trim();
        if !trimmed.is_empty() && !looks_like_openhuman(&hosts, trimmed) {
            // Derive a sensible default model from the legacy model_routes
            // (prefer "reasoning" hint, fall back to whatever is set).
            let default_model = config
                .model_routes
                .iter()
                .find(|r| r.hint.eq_ignore_ascii_case("reasoning"))
                .or_else(|| config.model_routes.first())
                .map(|r| r.model.clone())
                .filter(|m| !m.is_empty());
            config.cloud_providers.push(CloudProviderCreds {
                id: generate_provider_id("custom"),
                slug: "custom".to_string(),
                label: "Custom".to_string(),
                endpoint: trimmed.to_string(),
                auth_style: AuthStyle::Bearer,
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

/// Default `primary_cloud` to the legacy custom inference entry when one was
/// present; otherwise use the OpenHuman entry. This preserves pre-migration
/// behavior where a configured `inference_url` handled chat instead of the
/// hosted OpenHuman budget.
fn set_primary_cloud(config: &mut Config, stats: &mut MigrationStats) {
    if config.primary_cloud.is_some() {
        return;
    }
    let hosts = crate::config::enrichers::managed_hosts_for(config);
    let legacy_custom = config
        .inference_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty() && !looks_like_openhuman(&hosts, url))
        .and_then(|url| {
            let normalized = url.trim_end_matches('/').to_ascii_lowercase();
            config.cloud_providers.iter().find(|entry| {
                entry.slug != "openhuman"
                    && entry
                        .endpoint
                        .trim()
                        .trim_end_matches('/')
                        .to_ascii_lowercase()
                        == normalized
            })
        });
    if let Some(entry) = legacy_custom {
        config.primary_cloud = Some(entry.id.clone());
        stats.primary_cloud_set = true;
        log::debug!(
            "[migrations][unify-ai] primary_cloud set to legacy custom entry id={}",
            entry.id
        );
        return;
    }

    let oh = config
        .cloud_providers
        .iter()
        .find(|e| e.slug == "openhuman");
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
    let embed_model = config.local_ai.embedding_model_id.clone();

    let set_field = |field: &mut Option<String>, value: String, stats: &mut MigrationStats| {
        if field.is_none() {
            *field = Some(value);
            stats.workload_fields_filled += 1;
        }
    };

    // Summarisation workload: defaults to cloud.
    set_field(&mut config.memory_provider, "cloud".to_string(), stats);

    // Embeddings — uses the embedding_model_id, not chat_model_id.
    let embeddings_value =
        if config.local_ai.usage.embeddings && runtime_on && !embed_model.is_empty() {
            format!("ollama:{}", embed_model)
        } else {
            "cloud".to_string()
        };
    set_field(&mut config.embeddings_provider, embeddings_value, stats);

    // The three chat workloads (reasoning/agentic/coding) intentionally
    // stay None — the factory treats unset as "cloud" which routes to
    // primary_cloud. No equivalent of "force this to local" existed in the
    // legacy config for chat, so there's nothing to derive.
}

/// Heuristic: does the URL look like a configured managed backend?
///
/// Used to decide whether a non-empty `inference_url` should be migrated
/// into a Custom cloud provider entry. `hosts` are the managed-host patterns
/// the installed host resolved (`ResolvedEndpoints::managed_hosts`); with none
/// registered nothing matches.
///
/// Matches only on the host component to avoid false positives from custom
/// endpoints that happen to contain a managed name in a path or query string.
fn looks_like_openhuman(hosts: &[String], url: &str) -> bool {
    crate::config::schema::resolved_endpoints::host_matches_any(url, hosts)
}

#[cfg(test)]
#[path = "unify_ai_provider_settings_tests.rs"]
mod tests;
