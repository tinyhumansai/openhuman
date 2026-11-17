//! Product model configuration and runtime overlays over the owner price catalogue.
//!
//! Published prices, windows and id matching live in TinyAgents registry. The
//! host retains config enrichment, source attribution and discovered local models.

use crate::config::schema::ModelRegistryEntry;
use tinyagents_registry::pricing::{
    lookup, model_accepts_image_input, ModelPrice, KNOWN_MODEL_PRICING, PRICING_AS_OF,
};

const TINYAGENTS_CATALOG_SOURCE: &str = "openhuman-cost-catalog";

/// Build a default registry, one [`ModelRegistryEntry`] per catalogued model
/// with prices and context window pre-filled. Used to seed an empty
/// `config.model_registry`.
pub fn default_registry_entries() -> Vec<ModelRegistryEntry> {
    KNOWN_MODEL_PRICING
        .iter()
        .map(|p| ModelRegistryEntry {
            id: p.model_id.to_string(),
            provider: p.provider.to_string(),
            cost_per_1m_input: p.input_per_mtok_usd,
            cost_per_1m_cached_input: p.cached_input_per_mtok_usd,
            cost_per_1m_output: p.output_per_mtok_usd,
            context_window: p.context_window,
            vision: model_accepts_image_input(p.model_id),
        })
        .collect()
}

/// Project an owner price row as flat metadata with this product source label.
/// This preserves historical metadata, including its distinction from M3 tier estimates.
pub fn tinyagents_catalog_entry(price: &ModelPrice) -> tinyagents_registry::ModelCatalogEntry {
    tinyagents_registry::pricing::flat_catalog_entry(price, TINYAGENTS_CATALOG_SOURCE)
}

/// Source tag for local (runtime-discovered) model entries in the unified
/// catalog. Distinct from [`TINYAGENTS_CATALOG_SOURCE`] so consumers can tell a
/// priced vendor row apart from a free local runtime model.
const TINYAGENTS_LOCAL_SOURCE: &str = "openhuman-local-runtime";

/// A local model discovered at runtime (e.g. an installed Ollama tag) to overlay
/// onto the unified catalog.
///
/// Local runtimes are enumerated at runtime, not from any static table, so the
/// caller supplies these. Pricing is intentionally left unset (a local model is
/// not billed per-token); only identity, context window, and the runtime's
/// capability flags are carried.
#[derive(Debug, Clone)]
pub struct LocalCatalogModel {
    /// Local provider slug (e.g. `"ollama"`, `"lmstudio"`, `"mlx"`).
    pub provider: String,
    /// Concrete local model id / tag (e.g. `"qwen3:14b"`).
    pub model_id: String,
    /// Loaded/declared context window in tokens, when known. `None` falls back to
    /// the pattern-window backfill in [`unified_model_catalog`].
    pub context_window: Option<u64>,
    /// Whether the runtime advertises native tool calling for this model.
    pub tool_calling: bool,
    /// Whether the runtime streams tokens.
    pub streaming: bool,
}

/// Project one runtime-discovered local model into a TinyAgents catalog entry.
fn local_catalog_entry(model: &LocalCatalogModel) -> tinyagents_registry::ModelCatalogEntry {
    tinyagents_registry::ModelCatalogEntry {
        provider: model.provider.clone(),
        model_id: model.model_id.clone(),
        aliases: Vec::new(),
        mode: "chat".to_string(),
        max_input_tokens: model.context_window,
        max_output_tokens: None,
        deprecation_date: None,
        release_date: None,
        // Local runtimes are not billed per token; leave every price unset (not
        // zero — `None` means "not applicable", not "free of charge").
        pricing: tinyagents_harness::cost::ModelPricing::default(),
        capabilities: tinyagents_registry::ModelCapabilities {
            streaming: model.streaming,
            tool_calling: model.tool_calling,
            ..tinyagents_registry::ModelCapabilities::default()
        },
        source: TINYAGENTS_LOCAL_SOURCE.to_string(),
        source_url: None,
        raw: serde_json::json!({}),
    }
}

/// Upsert `entry` into `models` keyed by `(provider, model_id)`: replace an
/// existing row for that key, otherwise append. Later overlays win.
fn upsert_catalog_entry(
    models: &mut Vec<tinyagents_registry::ModelCatalogEntry>,
    entry: tinyagents_registry::ModelCatalogEntry,
) {
    if let Some(existing) = models
        .iter_mut()
        .find(|m| m.provider == entry.provider && m.model_id == entry.model_id)
    {
        *existing = entry;
    } else {
        models.push(entry);
    }
}

/// Build the single unified model catalog snapshot.
///
/// This is the **one** catalog projection consumers point at for pricing,
/// context windows, and capability flags. It is assembled by layering sources in
/// increasing precedence, so a later layer overrides an earlier one for the same
/// `(provider, model_id)`:
///
/// 1. **Crate seed** — `tinyagents_registry::ModelCatalog::seed()`, the crate's
///    checked-in offline catalog. This is the base/fallback set.
/// 2. **OpenHuman static rows** — [`KNOWN_MODEL_PRICING`], projected via
///    [`tinyagents_catalog_entry`]. OpenHuman's published rates/windows are
///    authoritative for the models the product routes to, so they overwrite any
///    crate-seed row for the same model. This is what keeps cost numbers
///    identical: the priced rows are the exact same `KNOWN_MODEL_PRICING` values
///    the cost bridge reads, only reshaped into crate entries.
/// 3. **Local runtime models** — `local_models` (e.g. installed Ollama tags),
///    appended (or overriding) as free, per-runtime entries.
///
/// After layering, any entry still missing a context window is backfilled from
/// [`crate::inference::model_context::context_window_for_model`] (which
/// itself consults [`KNOWN_MODEL_PRICING`] then the pattern-window fallbacks) —
/// this folds the `model_context.rs` pattern table into the one projection
/// without inventing windows for rows a source already declared.
///
/// Cost note: this snapshot is a *superset projection* of the cost catalog, not a
/// competing pricing source. `estimate_cost_usd` / `lookup` still read
/// [`KNOWN_MODEL_PRICING`] directly (their normalization + longest-substring
/// matching is not reproducible through the crate's exact-id lookup), so cost
/// estimates are unchanged; deleting `KNOWN_MODEL_PRICING` in favour of a
/// snapshot lookup is deferred until that lookup is proven numerically identical.
pub fn unified_model_catalog(
    local_models: &[LocalCatalogModel],
) -> tinyagents_registry::ModelCatalogSnapshot {
    // 1. Crate seed as the base layer.
    let (mut models, mut sources) = match tinyagents_registry::ModelCatalog::seed() {
        Ok(catalog) => {
            let snapshot = catalog.snapshot();
            (snapshot.models.clone(), snapshot.sources.clone())
        }
        Err(err) => {
            tracing::warn!(
                error = %err,
                "[cost][catalog] tinyagents crate seed failed to load; unified catalog omits crate-seed rows"
            );
            (Vec::new(), Vec::new())
        }
    };

    // 2. Overlay OpenHuman authoritative rates/windows (OpenHuman wins).
    for price in KNOWN_MODEL_PRICING {
        upsert_catalog_entry(&mut models, tinyagents_catalog_entry(price));
    }

    // 3. Overlay runtime-discovered local models.
    for local in local_models {
        upsert_catalog_entry(&mut models, local_catalog_entry(local));
    }

    // 4. Backfill still-missing context windows from the pattern fallbacks.
    //    Only fills `None` — never overwrites a window a source already set.
    for entry in models.iter_mut() {
        if entry.max_input_tokens.is_none() {
            if let Some(window) =
                crate::inference::model_context::context_window_for_model(&entry.model_id)
            {
                entry.max_input_tokens = Some(window);
            }
        }
    }

    // Record OpenHuman's own provenance alongside the crate seed's sources.
    sources.push(tinyagents_registry::ModelCatalogSource {
        name: TINYAGENTS_CATALOG_SOURCE.to_string(),
        url: "repo:crates/openhuman-core/src/platform/cost/catalog.rs".to_string(),
        retrieved_at: format!("{PRICING_AS_OF}-01T00:00:00Z"),
    });

    tinyagents_registry::ModelCatalogSnapshot {
        schema_version: 1,
        snapshot_id: format!("{TINYAGENTS_CATALOG_SOURCE}-unified-{PRICING_AS_OF}"),
        created_at: format!("{PRICING_AS_OF}-01T00:00:00Z"),
        currency: "USD".to_string(),
        unit: "token".to_string(),
        description: Some(
            "Unified OpenHuman model catalog: crate seed overlaid with OpenHuman cost/window rows and runtime-discovered local models.".to_string(),
        ),
        sources,
        models,
    }
}

/// Convenience wrapper: the unified catalog with **no** runtime-discovered local
/// models. Callers that cannot enumerate local runtimes (no config/network in
/// hand) use this; callers that can pass discovered models to
/// [`unified_model_catalog`] directly.
pub fn tinyagents_catalog_snapshot() -> tinyagents_registry::ModelCatalogSnapshot {
    unified_model_catalog(&[])
}

/// Pre-fill any **missing** (zero) price or context-window field on a registry
/// entry from the catalog, matching on its `id`. Leaves user-supplied non-zero
/// values. Catalogued image-capable models also receive `vision: true` when
/// the entry has the default false value. Returns `true` when a field was
/// filled in.
pub fn enrich_entry(entry: &mut ModelRegistryEntry) -> bool {
    let Some(price) = lookup(&entry.id) else {
        return false;
    };
    let mut changed = false;
    if entry.cost_per_1m_input == 0.0 {
        entry.cost_per_1m_input = price.input_per_mtok_usd;
        changed = true;
    }
    if entry.cost_per_1m_cached_input == 0.0 {
        entry.cost_per_1m_cached_input = price.cached_input_per_mtok_usd;
        changed = true;
    }
    if entry.cost_per_1m_output == 0.0 {
        entry.cost_per_1m_output = price.output_per_mtok_usd;
        changed = true;
    }
    if entry.context_window == 0 {
        entry.context_window = price.context_window;
        changed = true;
    }
    if !entry.vision && model_accepts_image_input(price.model_id) {
        entry.vision = true;
        changed = true;
    }
    changed
}

#[cfg(test)]
#[path = "catalog_tests.rs"]
mod tests;
