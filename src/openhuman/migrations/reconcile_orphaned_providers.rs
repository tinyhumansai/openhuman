//! Migration 5 → 6: reconcile orphaned per-workload provider references.
//!
//! ## The problem
//!
//! Each LLM workload is routed by a provider string in `Config`
//! (`chat_provider`, `reasoning_provider`, …) using the grammar parsed by
//! [`crate::openhuman::inference::provider::factory`]:
//!
//! ```text
//! ""/"cloud"          → primary_cloud (managed)
//! "openhuman"         → managed OpenHuman backend
//! "ollama:<model>"    → local Ollama
//! "lmstudio:<model>"  → local LM Studio
//! "<slug>:<model>"    → the cloud_providers entry whose slug == <slug>
//! ```
//!
//! A `"<slug>:<model>"` string only resolves while a `cloud_providers` entry
//! with that slug still exists. When a user **removes/disables a cloud
//! provider** (e.g. OpenAI) on one build, the entry leaves `cloud_providers`
//! but the workload string keeps pointing at it — e.g. `chat_provider =
//! "openai:gpt-4o"` with no `openai` entry. Nothing reconciles this, so:
//!
//! - the AI settings panel still shows the now-gone OpenAI model for chat, and
//! - at runtime [`factory::make_cloud_provider_by_slug`] **hard-errors**
//!   ("no cloud provider configured for slug 'openai'") instead of falling
//!   back to managed — that workload's inference breaks.
//!
//! ## What this migration does
//!
//! A pure, idempotent pass over the persisted `Config`:
//!
//! - For each of the nine `*_provider` fields, reset to `None` (= managed) any
//!   value that the factory could not resolve: a `"<slug>:<model>"` whose slug
//!   is absent from `cloud_providers`, the always-managed `"openhuman:<model>"`
//!   form, or a bare non-sentinel string. Sentinels (`""`, `"cloud"`,
//!   `"openhuman"`) and local providers (`ollama:`/`lmstudio:`, valid without a
//!   `cloud_providers` entry) are left untouched.
//! - Clear `primary_cloud` when it points at an id no longer present in
//!   `cloud_providers` (a dangling pointer; the factory already falls back to
//!   the managed backend for an unresolved primary).
//!
//! Mirrors the factory's *exact*, case-sensitive grammar so "resolvable here"
//! means the same as "resolvable at inference time". The one intentional step
//! beyond the factory is normalizing `"openhuman:<model>"` → `None`: both route
//! to the managed backend, and `None` matches what the settings UI persists.
//!
//! ## Behaviour
//!
//! - Pure in-memory mutation of `Config`; the caller (`migrations::run_pending`)
//!   persists via `Config::save()` and bumps `schema_version`.
//! - Idempotent: a second run finds nothing left to scrub.
//! - Never touches keys/secrets or any other config field.

use crate::openhuman::config::Config;
use crate::openhuman::inference::provider::factory::{
    LM_STUDIO_PROVIDER_PREFIX, OLLAMA_PROVIDER_PREFIX, PROVIDER_OPENHUMAN,
};
use std::collections::HashSet;

/// Counters returned by [`run`] for diagnostics. Logged at INFO once per run.
#[derive(Debug, Default, Clone)]
pub struct MigrationStats {
    /// Number of `*_provider` fields reset to managed (`None`).
    pub workload_fields_scrubbed: usize,
    /// `true` when a dangling `primary_cloud` pointer was cleared.
    pub primary_cloud_cleared: bool,
}

/// Run the orphaned-provider reconciliation on the given `Config`.
///
/// Synchronous — pure config mutation, no I/O. Caller persists via
/// `Config::save()` once `schema_version` is also bumped.
///
/// Returns `anyhow::Result` for uniformity with the other migration steps in
/// [`super`] (the runner matches `Ok`/`Err` for every step). This pass has no
/// fallible operations today, so it always returns `Ok`; the signature keeps
/// the contract consistent and leaves room for future I/O-bearing logic.
pub fn run(config: &mut Config) -> anyhow::Result<MigrationStats> {
    let mut stats = MigrationStats::default();

    // Own the slug/id sets so the immutable borrow on `cloud_providers` ends
    // before we take `&mut` to the workload fields below. Stored slugs/ids are
    // kept verbatim (no trim) so membership matches the factory's exact,
    // case-sensitive `e.slug == slug` / `e.id == id` comparisons: a provider
    // stored as " openai" must NOT make a "openai:model" route look resolvable,
    // since the factory would still fail it.
    let known_slugs: HashSet<String> = config
        .cloud_providers
        .iter()
        .map(|e| e.slug.clone())
        .collect();
    let known_ids: HashSet<String> = config
        .cloud_providers
        .iter()
        .map(|e| e.id.clone())
        .collect();

    let mut scrubbed = 0usize;
    for (workload, field) in workload_fields(config) {
        let Some(raw) = field.as_deref() else {
            continue;
        };
        let s = raw.trim();

        // Managed sentinels and local providers resolve without a
        // cloud_providers entry — leave them alone.
        if s.is_empty()
            || s == "cloud"
            || s == PROVIDER_OPENHUMAN
            || s.starts_with(OLLAMA_PROVIDER_PREFIX)
            || s.starts_with(LM_STUDIO_PROVIDER_PREFIX)
        {
            continue;
        }

        let scrub_reason = match s.split_once(':') {
            // "openhuman:<model>" is always the managed backend regardless of
            // the suffix — normalize to None to match the bare sentinel.
            Some((slug, _)) if slug.trim() == PROVIDER_OPENHUMAN => Some("openhuman-slug"),
            // "<slug>:<model>" whose slug is no longer configured — the orphan.
            Some((slug, _)) if !known_slugs.contains(slug.trim()) => Some("missing-slug"),
            Some(_) => None,
            // Bare non-sentinel (e.g. "openai") — the factory rejects this form.
            None => Some("bare-unresolvable"),
        };

        if let Some(reason) = scrub_reason {
            log::info!(
                "[migrations][reconcile-providers] {workload}_provider={} -> managed (None) \
                 reason={reason}",
                redact_provider_for_log(raw)
            );
            *field = None;
            scrubbed += 1;
        }
    }
    stats.workload_fields_scrubbed = scrubbed;

    // A primary_cloud id absent from cloud_providers is a dangling pointer.
    // The factory already resolves an unfound primary to the managed backend,
    // so null it for consistency with the on-disk truth.
    let dangling_primary = config
        .primary_cloud
        .as_deref()
        .is_some_and(|id| !known_ids.contains(id));
    if dangling_primary {
        // Don't log the raw id — primary_cloud is user-editable config; the fact
        // that it dangled is the only diagnostic that matters here.
        log::info!(
            "[migrations][reconcile-providers] primary_cloud points at a missing cloud_providers \
             entry -> cleared (managed fallback)"
        );
        config.primary_cloud = None;
        stats.primary_cloud_cleared = true;
    }

    log::info!(
        "[migrations][reconcile-providers] done workload_fields_scrubbed={} primary_cloud_cleared={}",
        stats.workload_fields_scrubbed,
        stats.primary_cloud_cleared
    );
    Ok(stats)
}

/// The nine per-workload routing fields, paired with a label for logging.
///
/// Borrowed mutably in one array literal — legal because they are disjoint
/// fields of `Config`.
///
/// IMPORTANT: this list must stay in sync with the `*_provider` fields on
/// [`Config`]. If a tenth workload provider field is added there, add it here
/// (and bump the array length) or the migration will silently skip it. Rust
/// can't enforce this at compile time without a field-reflection macro, and a
/// serde-based count guard doesn't work because the `Option<String>` fields
/// default to `None` and are omitted from the serialized table.
fn workload_fields(config: &mut Config) -> [(&'static str, &mut Option<String>); 9] {
    [
        ("chat", &mut config.chat_provider),
        ("reasoning", &mut config.reasoning_provider),
        ("agentic", &mut config.agentic_provider),
        ("coding", &mut config.coding_provider),
        ("memory", &mut config.memory_provider),
        ("embeddings", &mut config.embeddings_provider),
        ("heartbeat", &mut config.heartbeat_provider),
        ("learning", &mut config.learning_provider),
        ("subconscious", &mut config.subconscious_provider),
    ]
}

/// Redact a provider string for logging. Provider values are user-editable and
/// could carry sensitive content in the model segment, so keep only the
/// non-sensitive slug (the useful diagnostic) and mask the rest. Per the
/// project rule: never log secrets or full PII.
fn redact_provider_for_log(raw: &str) -> String {
    match raw.trim().split_once(':') {
        Some((slug, _)) => format!("{}:<redacted>", slug.trim()),
        None => "<redacted>".to_string(),
    }
}

#[cfg(test)]
#[path = "reconcile_orphaned_providers_tests.rs"]
mod tests;
