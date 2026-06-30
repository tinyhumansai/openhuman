//! Migration 6 → 7: retire the removed `"fastembed"` embedding provider.
//!
//! ## The problem
//!
//! Older builds shipped a local `"fastembed"` embedding provider (BGE models,
//! 384 dims). It has since been removed from the binary entirely — it is not a
//! cargo feature, it simply no longer exists in
//! [`crate::openhuman::embeddings::factory::create_embedding_provider`], which
//! hard-errors on any unknown provider string.
//!
//! Users who selected (or defaulted to) `"fastembed"` on an older build keep
//! `embedding_provider = "fastembed"` in their persisted `config.toml`. On
//! upgrade, the channel runtime's memory store build calls the factory with
//! that stale value → `Err("unknown embedding provider: \"fastembed\"")` →
//! `start_channels` aborts → **all messaging channels (Telegram/Discord) go
//! offline** with no surfaced error (issue #3712).
//!
//! ## What this migration does
//!
//! A pure, idempotent mutation of the persisted `Config`: when
//! `memory.embedding_provider` is the removed `"fastembed"` value, rewrite it to
//! a still-supported provider and reset the model/dimensions accordingly, since
//! the legacy BGE model/384-dim values are incompatible with both targets.
//!
//! **Target selection (offline-aware).** `fastembed` was a *local* embedder, so
//! a config carrying it belonged to a user who chose local/offline embeddings.
//! To preserve that intent, the caller (`run_pending`) probes for a reachable
//! local Ollama server and passes `prefer_local`:
//! - `prefer_local = true`  → `"ollama"` + `bge-m3` (1024-dim; Ollama auto-pulls
//!   the model on first embed). Keeps the user local/offline.
//! - `prefer_local = false` → `"managed"` cloud backend + cloud defaults (the
//!   fresh-install default), used when no local Ollama is reachable.
//!
//! Both targets are 1024-dim — matching the memory tree's fixed on-disk
//! `EMBEDDING_DIM=1024`. This step only ever rewrites `fastembed`, so cloud-only
//! users (never on `fastembed`) are untouched.
//!
//! Stored vectors written at the old signature are left in place: they are
//! ignored by signature-filtered vector search and re-generated lazily by the
//! existing re-embed backfill ([`crate::openhuman::memory_queue::ensure_reembed_backfill`])
//! once memory next syncs. No DB surgery happens here — this mirrors the
//! pure-config-mutation contract of the other migration steps.
//!
//! ## Behaviour
//!
//! - `run` is a **pure, synchronous** in-memory mutation of `Config`; the caller
//!   (`migrations::run_pending`) persists via `Config::save()` and bumps
//!   `schema_version`. The (impure) reachability probe lives in
//!   [`local_ollama_reachable`] and is invoked by the caller, not by `run`.
//! - Idempotent: once rewritten the provider is no longer `"fastembed"`, so a
//!   second run is a no-op.
//! - Never touches keys/secrets or any other config field.

use crate::openhuman::config::Config;
use crate::openhuman::embeddings::{
    DEFAULT_CLOUD_EMBEDDING_DIMENSIONS, DEFAULT_CLOUD_EMBEDDING_MODEL, DEFAULT_OLLAMA_DIMENSIONS,
    DEFAULT_OLLAMA_MODEL,
};

/// The removed provider value that must not reach the embedding factory.
const REMOVED_PROVIDER: &str = "fastembed";

/// Managed cloud backend — the current fresh-install default. Used as the
/// rewrite target when no local Ollama is reachable. Matches
/// `create_embedding_provider`'s accepted name.
const MANAGED_PROVIDER: &str = "managed";

/// Local Ollama backend — the rewrite target when a local Ollama server is
/// reachable, preserving the offline intent of a `fastembed` config. Matches
/// `create_embedding_provider`'s accepted name.
const OLLAMA_PROVIDER: &str = "ollama";

/// Counters returned by [`run`] for diagnostics. Logged at INFO once per run.
#[derive(Debug, Default, Clone)]
pub struct MigrationStats {
    /// Whether the removed `"fastembed"` provider was rewritten.
    pub provider_migrated: bool,
    /// Whether the rewrite target was local Ollama (`true`) vs managed cloud
    /// (`false`). Only meaningful when `provider_migrated`.
    pub migrated_to_local: bool,
    /// Embedding dimensionality before the rewrite (for the log line).
    pub old_dimensions: usize,
    /// Embedding dimensionality after the rewrite.
    pub new_dimensions: usize,
}

/// Rewrite a persisted `"fastembed"` embedding provider to a still-supported
/// backend.
///
/// `prefer_local` selects the target (see the module docs): `true` ⇒ local
/// Ollama (`bge-m3`, preserving offline intent), `false` ⇒ managed cloud. The
/// caller derives `prefer_local` from [`local_ollama_reachable`].
///
/// Synchronous — pure config mutation, no I/O. Caller persists via
/// `Config::save()` once `schema_version` is also bumped.
///
/// Returns `anyhow::Result` for uniformity with the other migration steps in
/// [`super`]; this pass has no fallible operations today and always returns
/// `Ok`.
pub fn run(config: &mut Config, prefer_local: bool) -> anyhow::Result<MigrationStats> {
    let mut stats = MigrationStats {
        old_dimensions: config.memory.embedding_dimensions,
        new_dimensions: config.memory.embedding_dimensions,
        ..Default::default()
    };

    if !config
        .memory
        .embedding_provider
        .trim()
        .eq_ignore_ascii_case(REMOVED_PROVIDER)
    {
        log::debug!(
            "[migrations][legacy-embedding] embedding_provider is not the removed \
             \"{REMOVED_PROVIDER}\" — nothing to do"
        );
        return Ok(stats);
    }

    // Both targets are 1024-dim (the memory tree's fixed on-disk EMBEDDING_DIM);
    // the legacy 384-dim BGE values are incompatible with either, so stored
    // vectors re-embed lazily via backfill regardless of which target we pick.
    let (provider, model, dimensions) = if prefer_local {
        (
            OLLAMA_PROVIDER,
            DEFAULT_OLLAMA_MODEL,
            DEFAULT_OLLAMA_DIMENSIONS,
        )
    } else {
        (
            MANAGED_PROVIDER,
            DEFAULT_CLOUD_EMBEDDING_MODEL,
            DEFAULT_CLOUD_EMBEDDING_DIMENSIONS,
        )
    };

    config.memory.embedding_provider = provider.to_string();
    config.memory.embedding_model = model.to_string();
    config.memory.embedding_dimensions = dimensions;

    stats.provider_migrated = true;
    stats.migrated_to_local = prefer_local;
    stats.new_dimensions = dimensions;

    log::info!(
        "[migrations][legacy-embedding] embedding_provider \"{REMOVED_PROVIDER}\" -> \
         \"{provider}\" (model={model}, dims {} -> {}, local={prefer_local}); \
         stale vectors re-embed lazily via backfill",
        stats.old_dimensions,
        stats.new_dimensions,
    );

    Ok(stats)
}

/// Best-effort probe: is a local Ollama server reachable at `base_url`?
///
/// Invoked by [`super::run_pending`] to choose the [`run`] target — a reachable
/// local Ollama lets former-`fastembed` (i.e. local-embedding) users stay local
/// (`bge-m3`, auto-pulled on first embed) instead of being forced onto the
/// managed cloud backend. Bounded (1.5s) and non-fatal: any client-build error,
/// transport error, timeout, or non-2xx status ⇒ `false`, so the caller falls
/// back to managed. Kept out of [`run`] so the rewrite itself stays pure/sync.
pub(crate) async fn local_ollama_reachable(base_url: &str) -> bool {
    let url = format!("{}/api/tags", base_url.trim_end_matches('/'));
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(1500))
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            log::debug!("[migrations][legacy-embedding] ollama probe client build failed: {error}");
            return false;
        }
    };
    match client.get(&url).send().await {
        Ok(resp) => {
            let reachable = resp.status().is_success();
            log::debug!(
                "[migrations][legacy-embedding] ollama probe {url} -> {} (reachable={reachable})",
                resp.status()
            );
            reachable
        }
        Err(error) => {
            log::debug!("[migrations][legacy-embedding] ollama probe {url} failed: {error}");
            false
        }
    }
}

#[cfg(test)]
#[path = "migrate_legacy_embedding_provider_tests.rs"]
mod tests;
