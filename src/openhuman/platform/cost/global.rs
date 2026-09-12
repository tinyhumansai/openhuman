//! Process-global `CostTracker` singleton.
//!
//! The dashboard RPC handlers and agent-turn telemetry hook share a single
//! tracker instance so cost records are persisted exactly once per provider
//! call and the in-memory daily/monthly aggregates stay coherent.
//!
//! Initialisation is intentionally lazy from the caller's perspective: the
//! `bootstrap_core_runtime` path calls [`init_global`] at startup, and any
//! later call is a no-op. Callers that run before bootstrap (e.g. unit
//! tests) see `None` from [`try_global`] and skip recording — never a panic.

use std::path::Path;
use std::sync::Arc;

use once_cell::sync::OnceCell;

use crate::openhuman::config::CostConfig;
use crate::openhuman::inference::provider::types::UsageInfo;

use super::tracker::CostTracker;
use super::types::{CostSource, TokenUsage};

static GLOBAL_TRACKER: OnceCell<Arc<CostTracker>> = OnceCell::new();

/// Initialise the global cost tracker. Idempotent — subsequent calls are
/// no-ops and the original tracker is preserved. Logs (but does not panic)
/// when construction fails so a bad workspace path never blocks core boot.
///
/// **Semantics note (changed in the cost-dashboard PR):**
///
/// - `cost.enabled = true` (the new default) — budget enforcement and
///   dashboard telemetry are both active.
/// - `cost.enabled = false` — budget enforcement is **off**, but the
///   dashboard telemetry path still appends to `costs.jsonl` (see
///   [`record_provider_usage`]). The flag now gates enforcement only;
///   observability is independent. This is a deliberate trade-off so
///   operators can review historical spend before opting into hard
///   budget caps. A `warn` is emitted below so the change is visible
///   in logs for anyone upgrading from a prior build where
///   `cost.enabled = false` blocked recording too.
///
/// The first-boot `info` log records `enabled` and the resolved
/// workspace so the default-on behaviour shows up in startup logs for
/// existing deployments that omit the `[cost]` block.
pub fn init_global(config: CostConfig, workspace_dir: &Path) {
    if GLOBAL_TRACKER.get().is_some() {
        return;
    }
    let cost_enabled = config.enabled;
    match CostTracker::new(config, workspace_dir) {
        Ok(tracker) => match GLOBAL_TRACKER.set(Arc::new(tracker)) {
            Ok(()) => {
                log::info!(
                    "[cost] global CostTracker initialised at workspace {} (cost.enabled={}, \
                     dashboard telemetry always-on). Set cost.dashboard.enabled=false in \
                     config.toml to hide the panel.",
                    workspace_dir.display(),
                    cost_enabled
                );
                if !cost_enabled {
                    log::warn!(
                        "[cost] cost.enabled=false: budget enforcement is OFF, but dashboard \
                         telemetry will still append to costs.jsonl. This is a behavioural \
                         change from prior builds where cost.enabled=false also blocked \
                         recording. Set cost.dashboard.enabled=false to disable the panel; \
                         the JSONL is local and never leaves the workspace."
                    );
                }
            }
            Err(_) => {
                // Another caller won a concurrent init race; the original
                // tracker is kept. Avoid logging a misleading "initialised"
                // line — the winner already did so.
                log::debug!(
                    "[cost] global CostTracker already initialised by another caller; \
                     discarding duplicate instance"
                );
            }
        },
        Err(err) => {
            log::warn!(
                "[cost] failed to initialise global CostTracker at {}: {err} \
                 — cost dashboard will report empty data until next core start",
                workspace_dir.display()
            );
        }
    }
}

/// Fetch the global tracker if it has been initialised. Returns `None`
/// before bootstrap or after an init failure — callers must treat the
/// absence as a soft no-op.
pub fn try_global() -> Option<Arc<CostTracker>> {
    GLOBAL_TRACKER.get().cloned()
}

/// Convenience hook used by the agent turn loop: translates a provider
/// [`UsageInfo`] into a [`TokenUsage`] record and persists it via the
/// global tracker. Silently skipped when the tracker is uninitialised.
/// Errors are logged but never propagated — cost tracking must never
/// break a turn.
///
/// Note: this path uses
/// [`crate::openhuman::platform::cost::tracker::CostTracker::record_usage_unconditional`],
/// so dashboard telemetry is captured even when `cost.enabled = false` —
/// the `cost.enabled` flag gates budget enforcement (refusing requests),
/// not observability. This lets users see history first and decide
/// whether to switch on enforcement.
///
/// `model` is the model identifier the request was routed to (e.g.
/// `"anthropic/claude-sonnet-4-20250514"`) and is used as the bucket key
/// in per-model aggregates.
pub fn record_provider_usage(model: &str, usage: &UsageInfo) {
    let Some(token_usage) = build_token_usage(model, usage) else {
        return;
    };
    let Some(tracker) = try_global() else {
        return;
    };
    if let Err(err) = tracker.record_usage_unconditional(token_usage) {
        log::debug!("[cost] record_provider_usage failed: {err}");
    }
}

/// Translate a provider [`UsageInfo`] into a [`TokenUsage`] record.
///
/// Returns `None` for an all-zero payload so the caller can skip the
/// write — providers that don't echo usage produce `UsageInfo::default()`
/// values, and persisting those would inflate the request count with
/// non-events. Non-finite or negative cost is clamped to `0.0`. Extracted
/// from [`record_provider_usage`] so the translation can be unit-tested
/// independently of the process-global tracker singleton.
pub(super) fn build_token_usage(model: &str, usage: &UsageInfo) -> Option<TokenUsage> {
    if usage.input_tokens == 0 && usage.output_tokens == 0 && usage.charged_amount_usd == 0.0 {
        return None;
    }
    let total_tokens = usage.input_tokens.saturating_add(usage.output_tokens);
    let provider_charged = usage.charged_amount_usd.is_finite() && usage.charged_amount_usd > 0.0;
    Some(TokenUsage {
        model: model.to_string(),
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        total_tokens,
        cached_input_tokens: usage.cached_input_tokens.min(usage.input_tokens),
        cache_creation_tokens: usage.cache_creation_tokens,
        reasoning_tokens: usage.reasoning_tokens,
        cost_usd: if usage.charged_amount_usd.is_finite() && usage.charged_amount_usd >= 0.0 {
            usage.charged_amount_usd
        } else {
            0.0
        },
        cost_source: if provider_charged {
            CostSource::ProviderCharged
        } else {
            CostSource::Estimated
        },
        // Lineage groundwork (06-cost step 3): the provider-usage build site
        // does not yet carry a run_id/root_run_id from the observation stream.
        // Leave `None` until the run-tree rollup (06.3, gated) threads run
        // lineage through `record_provider_usage` / the observation bridge in
        // `tinyagents/observability.rs`.
        run_id: None,
        root_run_id: None,
        timestamp: chrono::Utc::now(),
    })
}

/// Best-effort embedding cost recording (06-cost step 4 / 09-embeddings step 4).
///
/// Emits a [`CostRecord`](super::types::CostRecord) for a successful embedding
/// batch, priced via the unified pricing catalog. The record uses the
/// `"<provider>/<model>"` model key so it matches the embedding `CostRecord`
/// shape used elsewhere (e.g. `voyage/voyage-3`).
///
/// Pricing resolution: [`catalog::estimate_cost_usd`] prices `input_tokens` at
/// the catalogued per-token input rate. Embedding models are frequently **not**
/// in the pricing catalog; in that case `estimate_cost_usd` returns `0.0` and
/// we record the usage with **zero** cost (and log it) rather than fabricating a
/// rate. Output tokens are always zero for embeddings.
///
/// This path is **non-fatal**: it must never fail an embed or a recall turn.
/// A missing global tracker, or a tracker write error, is logged under
/// `[cost][embed]` and swallowed.
///
/// - `provider` — embedding provider slug (e.g. `voyage`, `openai`, `ollama`).
/// - `model` — provider model id (e.g. `voyage-3`).
/// - `input_tokens` — approximate input token count for the batch.
/// - `dimensions` — embedding vector dimensionality (logging context only).
/// - `vector_count` — number of vectors produced (logging context only).
pub fn record_embedding_usage(
    provider: &str,
    model: &str,
    input_tokens: u64,
    dimensions: usize,
    vector_count: u64,
) {
    let Some(tracker) = try_global() else {
        log::debug!(
            "[cost][embed] tracker not initialised; skipping embedding cost record \
             provider={provider} model={model} vectors={vector_count}"
        );
        return;
    };
    let cost_usd = super::catalog::estimate_cost_usd(model, input_tokens, 0, 0);
    if cost_usd == 0.0 {
        log::debug!(
            "[cost][embed] no catalog embedding rate for model={model}; recording usage with \
             zero cost (provider={provider} dims={dimensions} vectors={vector_count} \
             input_tokens={input_tokens})"
        );
    }
    let usage = TokenUsage {
        // `<provider>/<model>` bucket key, matching the embedding CostRecord
        // shape used in the dashboard/RPC layer (e.g. `voyage/voyage-3`).
        model: format!("{provider}/{model}"),
        input_tokens,
        output_tokens: 0,
        total_tokens: input_tokens,
        cached_input_tokens: 0,
        cache_creation_tokens: 0,
        reasoning_tokens: 0,
        cost_usd,
        cost_source: CostSource::Estimated,
        run_id: None,
        root_run_id: None,
        timestamp: chrono::Utc::now(),
    };
    log::debug!(
        "[cost][embed] recording embedding usage provider={provider} model={model} \
         input_tokens={input_tokens} dims={dimensions} vectors={vector_count} cost_usd={cost_usd}"
    );
    if let Err(err) = tracker.record_usage_unconditional(usage) {
        log::debug!(
            "[cost][embed] record_embedding_usage failed provider={provider} model={model}: {err}"
        );
    }
}

#[cfg(test)]
#[path = "global_tests.rs"]
mod tests;
