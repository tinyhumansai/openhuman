//! Static pricing + context-window catalog for known LLM models.
//!
//! This is the single source of truth for **pre-filled** per-model metadata of
//! the default models the product can route to:
//!
//! - per-token pricing (input + cached-input + output, USD per million tokens),
//! - the model's **context window** (max input tokens) — different providers
//!   ship very different windows, and callers need it to budget prompts,
//!   trigger compaction, and route work.
//!
//! It exists so the client can estimate request cost and reason about context
//! limits for any provider, used to:
//!
//! - pre-fill [`crate::openhuman::config::schema::ModelRegistryEntry`] rows so
//!   the Model Health dashboard shows real numbers instead of zeros, and
//! - power the fallback estimate in
//!   [`crate::openhuman::agent::cost::lookup_pricing`] when a backend doesn't
//!   echo an authoritative `charged_amount_usd`.
//! - project OpenHuman's static rows into TinyAgents catalog entries so later
//!   phases can hydrate crate-native model profiles from the same pricing and
//!   window source instead of carrying a second table.
//!
//! ## Authority & freshness
//!
//! These are **best-effort published values** captured at [`PRICING_AS_OF`].
//! The provider-reported `charged_amount_usd` always wins for cost when
//! present; the catalog is only a floor estimate. Context windows are the
//! published maximums and may differ from what a given deployment/tier exposes.
//! Prices and windows drift — when a provider changes them or a new default
//! model ships, update the matching row here (and bump [`PRICING_AS_OF`]). The
//! table is intentionally a plain `const` slice with no I/O so it's cheap to
//! consult on every lookup.
//!
//! ## Matching
//!
//! [`lookup`] resolves a concrete model string to a row. It normalises case,
//! strips a leading `vendor/` segment (OpenRouter-style ids like
//! `anthropic/claude-opus-4-8`) and trailing decorations (`:tag`, `@date`,
//! `[1m]`), and finally does a longest-substring match so dated/suffixed ids
//! (`claude-opus-4-8[1m]`, `gpt-5.4-2026-05-01`) still resolve.

use crate::openhuman::config::schema::ModelRegistryEntry;

/// Month the published values below were last verified. Bump when refreshing.
pub const PRICING_AS_OF: &str = "2026-06";

const TINYAGENTS_CATALOG_SOURCE: &str = "openhuman-cost-catalog";

/// A single model's published per-million-token rates (USD) and context window.
#[derive(Debug, Clone, Copy)]
pub struct ModelPrice {
    /// Canonical provider slug, matching the `cloud_providers` type strings
    /// (`anthropic`, `openai`, `google`, `deepseek`, `moonshot`, `qwen`,
    /// `mistral`). Used as the `provider` field when pre-filling registry rows.
    pub provider: &'static str,
    /// Canonical, lower-case model id used for matching. Keep these distinctive
    /// (no bare `gpt-5`) so substring matching stays unambiguous.
    pub model_id: &'static str,
    /// USD per million standard (cache-miss) input tokens.
    pub input_per_mtok_usd: f64,
    /// USD per million cached-prefix input tokens. Best-effort: exact where the
    /// provider publishes it, otherwise the provider's typical cache discount.
    pub cached_input_per_mtok_usd: f64,
    /// USD per million output tokens.
    pub output_per_mtok_usd: f64,
    /// Maximum context window in tokens (published max input). Providers differ
    /// widely (128K–1M+); callers budget prompts / trigger compaction off this.
    pub context_window: u32,
}

/// Published list prices and context windows for the default models the product
/// can route to.
///
/// Sources (captured [`PRICING_AS_OF`]): vendor pricing/model pages. Anthropic
/// price rows are authoritative (cached = 0.1× input, the documented cache-read
/// rate); other providers' cached rates use the published discount where known
/// and a conservative provider-typical fraction otherwise. Context windows are
/// the published maximums.
const KNOWN_MODEL_PRICING: &[ModelPrice] = &[
    // ── Anthropic (authoritative prices; cache read = 0.1× input) ────────────
    ModelPrice {
        provider: "anthropic",
        model_id: "claude-fable-5",
        input_per_mtok_usd: 10.00,
        cached_input_per_mtok_usd: 1.00,
        output_per_mtok_usd: 50.00,
        context_window: 1_000_000,
    },
    ModelPrice {
        provider: "anthropic",
        model_id: "claude-opus-4-8",
        input_per_mtok_usd: 5.00,
        cached_input_per_mtok_usd: 0.50,
        output_per_mtok_usd: 25.00,
        context_window: 1_000_000,
    },
    ModelPrice {
        provider: "anthropic",
        model_id: "claude-opus-4-7",
        input_per_mtok_usd: 5.00,
        cached_input_per_mtok_usd: 0.50,
        output_per_mtok_usd: 25.00,
        context_window: 1_000_000,
    },
    ModelPrice {
        provider: "anthropic",
        model_id: "claude-opus-4-6",
        input_per_mtok_usd: 5.00,
        cached_input_per_mtok_usd: 0.50,
        output_per_mtok_usd: 25.00,
        context_window: 1_000_000,
    },
    ModelPrice {
        provider: "anthropic",
        model_id: "claude-opus-4-5",
        input_per_mtok_usd: 5.00,
        cached_input_per_mtok_usd: 0.50,
        output_per_mtok_usd: 25.00,
        context_window: 200_000,
    },
    ModelPrice {
        provider: "anthropic",
        model_id: "claude-sonnet-4-6",
        input_per_mtok_usd: 3.00,
        cached_input_per_mtok_usd: 0.30,
        output_per_mtok_usd: 15.00,
        context_window: 1_000_000,
    },
    ModelPrice {
        provider: "anthropic",
        model_id: "claude-sonnet-4-5",
        input_per_mtok_usd: 3.00,
        cached_input_per_mtok_usd: 0.30,
        output_per_mtok_usd: 15.00,
        context_window: 200_000,
    },
    ModelPrice {
        provider: "anthropic",
        model_id: "claude-haiku-4-5",
        input_per_mtok_usd: 1.00,
        cached_input_per_mtok_usd: 0.10,
        output_per_mtok_usd: 5.00,
        context_window: 200_000,
    },
    // ── OpenAI (cache read ≈ 0.25× input — published 75% off) ────────────────
    ModelPrice {
        provider: "openai",
        model_id: "gpt-5.5",
        input_per_mtok_usd: 5.00,
        cached_input_per_mtok_usd: 1.25,
        output_per_mtok_usd: 30.00,
        context_window: 1_000_000,
    },
    ModelPrice {
        provider: "openai",
        model_id: "gpt-5.4",
        input_per_mtok_usd: 2.50,
        cached_input_per_mtok_usd: 0.625,
        output_per_mtok_usd: 15.00,
        context_window: 1_000_000,
    },
    ModelPrice {
        provider: "openai",
        model_id: "gpt-5.4-mini",
        input_per_mtok_usd: 0.75,
        cached_input_per_mtok_usd: 0.1875,
        output_per_mtok_usd: 4.50,
        context_window: 400_000,
    },
    ModelPrice {
        provider: "openai",
        model_id: "gpt-5.4-nano",
        input_per_mtok_usd: 0.20,
        cached_input_per_mtok_usd: 0.05,
        output_per_mtok_usd: 1.25,
        context_window: 400_000,
    },
    ModelPrice {
        provider: "openai",
        model_id: "gpt-4.1",
        input_per_mtok_usd: 2.00,
        cached_input_per_mtok_usd: 0.50,
        output_per_mtok_usd: 8.00,
        context_window: 1_047_576,
    },
    ModelPrice {
        provider: "openai",
        model_id: "gpt-4.1-mini",
        input_per_mtok_usd: 0.40,
        cached_input_per_mtok_usd: 0.10,
        output_per_mtok_usd: 1.60,
        context_window: 1_047_576,
    },
    ModelPrice {
        provider: "openai",
        model_id: "o3",
        input_per_mtok_usd: 2.00,
        cached_input_per_mtok_usd: 0.50,
        output_per_mtok_usd: 8.00,
        context_window: 200_000,
    },
    // ── Google Gemini (cache read ≈ 0.25× input; 1M-token windows) ───────────
    ModelPrice {
        provider: "google",
        model_id: "gemini-2.5-pro",
        input_per_mtok_usd: 1.25,
        cached_input_per_mtok_usd: 0.3125,
        output_per_mtok_usd: 10.00,
        context_window: 1_048_576,
    },
    ModelPrice {
        provider: "google",
        model_id: "gemini-2.5-flash",
        input_per_mtok_usd: 0.30,
        cached_input_per_mtok_usd: 0.075,
        output_per_mtok_usd: 2.50,
        context_window: 1_048_576,
    },
    ModelPrice {
        provider: "google",
        model_id: "gemini-2.5-flash-lite",
        input_per_mtok_usd: 0.10,
        cached_input_per_mtok_usd: 0.025,
        output_per_mtok_usd: 0.40,
        context_window: 1_048_576,
    },
    // ── DeepSeek (cache hit = 0.1× input, published) ─────────────────────────
    ModelPrice {
        provider: "deepseek",
        model_id: "deepseek-chat",
        input_per_mtok_usd: 0.14,
        cached_input_per_mtok_usd: 0.014,
        output_per_mtok_usd: 0.28,
        context_window: 128_000,
    },
    ModelPrice {
        provider: "deepseek",
        model_id: "deepseek-reasoner",
        input_per_mtok_usd: 0.55,
        cached_input_per_mtok_usd: 0.055,
        output_per_mtok_usd: 2.19,
        context_window: 128_000,
    },
    // ── Moonshot Kimi (cache hit published) ──────────────────────────────────
    ModelPrice {
        provider: "moonshot",
        model_id: "kimi-k2.6",
        input_per_mtok_usd: 0.95,
        cached_input_per_mtok_usd: 0.16,
        output_per_mtok_usd: 4.00,
        context_window: 256_000,
    },
    ModelPrice {
        provider: "moonshot",
        model_id: "kimi-k2.5",
        input_per_mtok_usd: 0.60,
        cached_input_per_mtok_usd: 0.10,
        output_per_mtok_usd: 3.00,
        context_window: 256_000,
    },
    // ── Qwen / Alibaba (cache read ≈ 0.1× input) ─────────────────────────────
    ModelPrice {
        provider: "qwen",
        model_id: "qwen3-max",
        input_per_mtok_usd: 1.20,
        cached_input_per_mtok_usd: 0.12,
        output_per_mtok_usd: 6.00,
        context_window: 256_000,
    },
    ModelPrice {
        provider: "qwen",
        model_id: "qwen-max",
        input_per_mtok_usd: 1.20,
        cached_input_per_mtok_usd: 0.12,
        output_per_mtok_usd: 6.00,
        context_window: 256_000,
    },
    ModelPrice {
        provider: "qwen",
        model_id: "qwen-plus",
        input_per_mtok_usd: 0.40,
        cached_input_per_mtok_usd: 0.04,
        output_per_mtok_usd: 1.20,
        context_window: 256_000,
    },
    ModelPrice {
        provider: "qwen",
        model_id: "qwen-flash",
        input_per_mtok_usd: 0.05,
        cached_input_per_mtok_usd: 0.005,
        output_per_mtok_usd: 0.40,
        context_window: 256_000,
    },
    // ── Mistral (cache read ≈ 0.1× input) ────────────────────────────────────
    ModelPrice {
        provider: "mistral",
        model_id: "mistral-large",
        input_per_mtok_usd: 2.00,
        cached_input_per_mtok_usd: 0.20,
        output_per_mtok_usd: 6.00,
        context_window: 128_000,
    },
    ModelPrice {
        provider: "mistral",
        model_id: "mistral-medium",
        input_per_mtok_usd: 0.40,
        cached_input_per_mtok_usd: 0.04,
        output_per_mtok_usd: 2.00,
        context_window: 128_000,
    },
    ModelPrice {
        provider: "mistral",
        model_id: "mistral-small",
        input_per_mtok_usd: 0.20,
        cached_input_per_mtok_usd: 0.02,
        output_per_mtok_usd: 0.60,
        context_window: 128_000,
    },
    ModelPrice {
        provider: "mistral",
        model_id: "codestral",
        input_per_mtok_usd: 0.30,
        cached_input_per_mtok_usd: 0.03,
        output_per_mtok_usd: 0.90,
        context_window: 256_000,
    },
    ModelPrice {
        provider: "mistral",
        model_id: "ministral-8b",
        input_per_mtok_usd: 0.10,
        cached_input_per_mtok_usd: 0.01,
        output_per_mtok_usd: 0.10,
        context_window: 128_000,
    },
];

/// Normalise a model string for matching: lower-case, trim, drop a trailing
/// `:tag` / `@date` decoration and a `[...]` suffix.
fn normalize(model: &str) -> String {
    let mut s = model.trim().to_ascii_lowercase();
    // Strip a `[1m]`-style context-window suffix.
    if let Some(idx) = s.find('[') {
        s.truncate(idx);
    }
    // Strip `:tag` (e.g. ollama-style) and `@date` (Vertex-style) decorations.
    for sep in [':', '@'] {
        if let Some(idx) = s.find(sep) {
            s.truncate(idx);
        }
    }
    s.trim().to_string()
}

/// Resolve a concrete model string to its catalogued row, if known.
///
/// Match order: exact canonical id → id with a leading `vendor/` segment
/// stripped → longest canonical id that is a substring of the normalised
/// request (handles dated/suffixed ids). Returns `None` for unknown models —
/// callers should fall back to a tier estimate.
pub fn lookup(model: &str) -> Option<&'static ModelPrice> {
    let norm = normalize(model);
    if norm.is_empty() {
        return None;
    }
    if let Some(p) = KNOWN_MODEL_PRICING.iter().find(|p| p.model_id == norm) {
        return Some(p);
    }
    let bare = norm.rsplit('/').next().unwrap_or(norm.as_str());
    if let Some(p) = KNOWN_MODEL_PRICING.iter().find(|p| p.model_id == bare) {
        return Some(p);
    }
    KNOWN_MODEL_PRICING
        .iter()
        .filter(|p| {
            contains_at_boundary(&norm, p.model_id) || contains_at_boundary(bare, p.model_id)
        })
        .max_by_key(|p| p.model_id.len())
}

/// Whether `needle` occurs in `haystack` at a token boundary — i.e. the
/// characters immediately flanking the match are non-alphanumeric (or the string
/// ends). This keeps the longest-substring fallback matching dated/suffixed ids
/// (`gpt-5.4-mini-2026-05-01` → `gpt-5.4-mini`, flanked by `-`/string end) while
/// refusing spurious mid-token collisions for short canonical ids
/// (`proto3-chat` must NOT match the `o3` row, `solo1-7b` must NOT match `o1`).
/// A naive `contains` overmatches those and, since [`context_window_for_model`]
/// now routes windows through this catalog, that leaked a bogus 200K window
/// (issue #4249 regression guard).
fn contains_at_boundary(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let bytes = haystack.as_bytes();
    let mut search_from = 0;
    while let Some(rel) = haystack[search_from..].find(needle) {
        let start = search_from + rel;
        let end = start + needle.len();
        let before_ok = start == 0 || !bytes[start - 1].is_ascii_alphanumeric();
        let after_ok = end == haystack.len() || !bytes[end].is_ascii_alphanumeric();
        if before_ok && after_ok {
            return true;
        }
        search_from = start + 1;
    }
    false
}

/// Estimate the USD cost of a single model call from catalogued per-MTok rates.
///
/// Prices the standard (cache-miss) input tokens, the cached-prefix input
/// tokens, and the output tokens separately. `cached_input_tokens` are billed at
/// the (usually cheaper) cached rate and are assumed to be a subset of
/// `input_tokens`, so the standard-rate portion is `input − cached`. Returns
/// `0.0` when the model is not catalogued (caller should treat as "unknown, not
/// free" — this is a best-effort estimate used when the provider does not report
/// a charged amount).
pub fn estimate_cost_usd(
    model: &str,
    input_tokens: u64,
    output_tokens: u64,
    cached_input_tokens: u64,
) -> f64 {
    let Some(p) = lookup(model) else {
        return 0.0;
    };
    let cached = cached_input_tokens.min(input_tokens);
    let standard_input = input_tokens.saturating_sub(cached);
    let per_tok = |mtok_rate: f64| mtok_rate / 1_000_000.0;
    (standard_input as f64) * per_tok(p.input_per_mtok_usd)
        + (cached as f64) * per_tok(p.cached_input_per_mtok_usd)
        + (output_tokens as f64) * per_tok(p.output_per_mtok_usd)
}

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
            vision: false,
        })
        .collect()
}

fn per_token(rate_per_mtok: f64) -> Option<f64> {
    (rate_per_mtok > 0.0).then_some(rate_per_mtok / 1_000_000.0)
}

/// Project one OpenHuman catalog row into a TinyAgents model-catalog entry.
///
/// This is intentionally a pricing/window projection only. OpenHuman still
/// derives live runtime capability flags (tools, vision, streaming) from the
/// provider adapter at construction time, because the static cost catalog does
/// not yet encode those fields authoritatively for every provider/model.
pub fn tinyagents_catalog_entry(price: &ModelPrice) -> tinyagents::registry::ModelCatalogEntry {
    tinyagents::registry::ModelCatalogEntry {
        provider: price.provider.to_string(),
        model_id: price.model_id.to_string(),
        aliases: Vec::new(),
        mode: "chat".to_string(),
        max_input_tokens: Some(u64::from(price.context_window)),
        max_output_tokens: None,
        deprecation_date: None,
        pricing: tinyagents::registry::ModelPricing {
            input_per_token: per_token(price.input_per_mtok_usd),
            output_per_token: per_token(price.output_per_mtok_usd),
            cache_read_input_per_token: per_token(price.cached_input_per_mtok_usd),
            cache_creation_input_per_token: None,
            input_audio_per_token: None,
            output_reasoning_per_token: None,
        },
        capabilities: tinyagents::registry::ModelCapabilities {
            prompt_caching: price.cached_input_per_mtok_usd > 0.0,
            ..tinyagents::registry::ModelCapabilities::default()
        },
        source: TINYAGENTS_CATALOG_SOURCE.to_string(),
        source_url: None,
        raw: serde_json::json!({ "pricing_as_of": PRICING_AS_OF }),
    }
}

/// Resolve a model id and return its TinyAgents catalog projection.
pub fn tinyagents_catalog_entry_for_model(
    model: &str,
) -> Option<tinyagents::registry::ModelCatalogEntry> {
    lookup(model).map(tinyagents_catalog_entry)
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
fn local_catalog_entry(model: &LocalCatalogModel) -> tinyagents::registry::ModelCatalogEntry {
    tinyagents::registry::ModelCatalogEntry {
        provider: model.provider.clone(),
        model_id: model.model_id.clone(),
        aliases: Vec::new(),
        mode: "chat".to_string(),
        max_input_tokens: model.context_window,
        max_output_tokens: None,
        deprecation_date: None,
        // Local runtimes are not billed per token; leave every price unset (not
        // zero — `None` means "not applicable", not "free of charge").
        pricing: tinyagents::registry::ModelPricing::default(),
        capabilities: tinyagents::registry::ModelCapabilities {
            streaming: model.streaming,
            tool_calling: model.tool_calling,
            ..tinyagents::registry::ModelCapabilities::default()
        },
        source: TINYAGENTS_LOCAL_SOURCE.to_string(),
        source_url: None,
        raw: serde_json::json!({}),
    }
}

/// Upsert `entry` into `models` keyed by `(provider, model_id)`: replace an
/// existing row for that key, otherwise append. Later overlays win.
fn upsert_catalog_entry(
    models: &mut Vec<tinyagents::registry::ModelCatalogEntry>,
    entry: tinyagents::registry::ModelCatalogEntry,
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
/// 1. **Crate seed** — `tinyagents::registry::ModelCatalog::seed()`, the crate's
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
/// [`crate::openhuman::inference::model_context::context_window_for_model`] (which
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
) -> tinyagents::registry::ModelCatalogSnapshot {
    // 1. Crate seed as the base layer.
    let (mut models, mut sources) = match tinyagents::registry::ModelCatalog::seed() {
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
                crate::openhuman::inference::model_context::context_window_for_model(
                    &entry.model_id,
                )
            {
                entry.max_input_tokens = Some(window);
            }
        }
    }

    // Record OpenHuman's own provenance alongside the crate seed's sources.
    sources.push(tinyagents::registry::ModelCatalogSource {
        name: TINYAGENTS_CATALOG_SOURCE.to_string(),
        url: "repo:src/openhuman/cost/catalog.rs".to_string(),
        retrieved_at: format!("{PRICING_AS_OF}-01T00:00:00Z"),
    });

    tinyagents::registry::ModelCatalogSnapshot {
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
pub fn tinyagents_catalog_snapshot() -> tinyagents::registry::ModelCatalogSnapshot {
    unified_model_catalog(&[])
}

/// Pre-fill any **missing** (zero) price or context-window field on a registry
/// entry from the catalog, matching on its `id`. Leaves user-supplied non-zero
/// values and the `vision` flag untouched. Returns `true` when a field was
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
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_lookup_resolves_canonical_ids() {
        let p = lookup("claude-opus-4-8").expect("anthropic row");
        assert_eq!(p.provider, "anthropic");
        assert_eq!(p.input_per_mtok_usd, 5.00);
        assert_eq!(p.output_per_mtok_usd, 25.00);
        assert_eq!(p.cached_input_per_mtok_usd, 0.50);
        assert_eq!(p.context_window, 1_000_000);
    }

    #[test]
    fn lookup_is_case_insensitive() {
        assert_eq!(lookup("GPT-4.1").unwrap().model_id, "gpt-4.1");
    }

    #[test]
    fn lookup_strips_vendor_prefix_openrouter_style() {
        assert_eq!(
            lookup("anthropic/claude-sonnet-4-6").unwrap().model_id,
            "claude-sonnet-4-6"
        );
        assert_eq!(
            lookup("deepseek/deepseek-chat").unwrap().model_id,
            "deepseek-chat"
        );
        assert_eq!(lookup("qwen/qwen3-max").unwrap().model_id, "qwen3-max");
    }

    #[test]
    fn lookup_strips_context_and_tag_decorations() {
        assert_eq!(
            lookup("claude-opus-4-8[1m]").unwrap().model_id,
            "claude-opus-4-8"
        );
        assert_eq!(lookup("kimi-k2.6:turbo").unwrap().model_id, "kimi-k2.6");
        assert_eq!(
            lookup("claude-opus-4-5@20251101").unwrap().model_id,
            "claude-opus-4-5"
        );
    }

    #[test]
    fn lookup_longest_substring_wins_for_suffixed_ids() {
        // A dated/suffixed id should resolve to the most specific row.
        assert_eq!(
            lookup("gpt-5.4-mini-2026-05-01").unwrap().model_id,
            "gpt-5.4-mini"
        );
    }

    #[test]
    fn lookup_returns_none_for_unknown() {
        assert!(lookup("totally-made-up-model").is_none());
        assert!(lookup("").is_none());
        assert!(
            lookup("agentic-v1").is_none(),
            "abstract tiers aren't vendor models"
        );
    }

    #[test]
    fn default_registry_entries_are_fully_populated() {
        let entries = default_registry_entries();
        assert_eq!(entries.len(), KNOWN_MODEL_PRICING.len());
        for e in &entries {
            assert!(e.cost_per_1m_input > 0.0, "{} missing input price", e.id);
            assert!(e.cost_per_1m_output > 0.0, "{} missing output price", e.id);
            assert!(e.context_window > 0, "{} missing context window", e.id);
            assert!(!e.provider.is_empty());
        }
    }

    #[test]
    fn tinyagents_projection_uses_per_token_rates_and_context_window() {
        let entry = tinyagents_catalog_entry_for_model("anthropic/claude-opus-4-8")
            .expect("projected catalog entry");
        assert_eq!(entry.provider, "anthropic");
        assert_eq!(entry.model_id, "claude-opus-4-8");
        assert_eq!(entry.mode, "chat");
        assert_eq!(entry.max_input_tokens, Some(1_000_000));
        assert_eq!(entry.pricing.input_per_token, Some(5.0 / 1_000_000.0));
        assert_eq!(entry.pricing.output_per_token, Some(25.0 / 1_000_000.0));
        assert_eq!(
            entry.pricing.cache_read_input_per_token,
            Some(0.50 / 1_000_000.0)
        );
        assert_eq!(entry.pricing.cache_creation_input_per_token, None);
        assert_eq!(entry.pricing.output_reasoning_per_token, None);
        assert!(entry.capabilities.prompt_caching);
        assert_eq!(entry.source, TINYAGENTS_CATALOG_SOURCE);
    }

    #[test]
    fn tinyagents_snapshot_contains_all_known_rows() {
        let snapshot = tinyagents_catalog_snapshot();
        assert_eq!(snapshot.schema_version, 1);
        assert_eq!(snapshot.currency, "USD");
        assert_eq!(snapshot.unit, "token");
        // Unified snapshot is a superset (crate seed + OpenHuman overlay), so it
        // is at least as large as the OpenHuman table and carries every
        // OpenHuman row with its authoritative pricing/window.
        assert!(snapshot.models.len() >= KNOWN_MODEL_PRICING.len());
        for price in KNOWN_MODEL_PRICING {
            let entry = snapshot
                .models
                .iter()
                .find(|m| m.provider == price.provider && m.model_id == price.model_id)
                .unwrap_or_else(|| panic!("missing {} in unified snapshot", price.model_id));
            assert_eq!(
                entry.max_input_tokens,
                Some(u64::from(price.context_window))
            );
            assert_eq!(
                entry.pricing.input_per_token,
                Some(price.input_per_mtok_usd / 1_000_000.0)
            );
        }
        // OpenHuman provenance is recorded alongside any crate-seed sources.
        assert!(snapshot
            .sources
            .iter()
            .any(|s| s.name == TINYAGENTS_CATALOG_SOURCE));
    }

    #[test]
    fn unified_catalog_overlays_local_models() {
        let local = vec![LocalCatalogModel {
            provider: "ollama".to_string(),
            model_id: "qwen3:14b".to_string(),
            context_window: Some(32_768),
            tool_calling: true,
            streaming: true,
        }];
        let snapshot = unified_model_catalog(&local);
        let entry = snapshot
            .models
            .iter()
            .find(|m| m.provider == "ollama" && m.model_id == "qwen3:14b")
            .expect("local model present");
        assert_eq!(entry.max_input_tokens, Some(32_768));
        assert!(entry.capabilities.tool_calling);
        // Local runtime models are not billed per token.
        assert_eq!(entry.pricing.input_per_token, None);
        assert_eq!(entry.pricing.output_per_token, None);
        assert_eq!(entry.source, TINYAGENTS_LOCAL_SOURCE);
    }

    #[test]
    fn unified_catalog_backfills_missing_window_without_source_window() {
        // A local model with no declared window falls back to the pattern table
        // via `context_window_for_model` (deepseek pattern → 128k) instead of
        // staying unbounded.
        let local = vec![LocalCatalogModel {
            provider: "ollama".to_string(),
            model_id: "deepseek-r1:7b".to_string(),
            context_window: None,
            tool_calling: false,
            streaming: true,
        }];
        let snapshot = unified_model_catalog(&local);
        let entry = snapshot
            .models
            .iter()
            .find(|m| m.provider == "ollama" && m.model_id == "deepseek-r1:7b")
            .expect("local model present");
        assert_eq!(entry.max_input_tokens, Some(128_000));
    }

    #[test]
    fn enrich_fills_zeros_but_preserves_user_values() {
        let mut e = ModelRegistryEntry {
            id: "claude-opus-4-8".to_string(),
            provider: "anthropic".to_string(),
            cost_per_1m_input: 0.0,
            cost_per_1m_cached_input: 0.0,
            cost_per_1m_output: 99.0, // user override — must survive
            context_window: 0,
            vision: true,
        };
        assert!(enrich_entry(&mut e));
        assert_eq!(e.cost_per_1m_input, 5.00);
        assert_eq!(e.cost_per_1m_cached_input, 0.50);
        assert_eq!(e.cost_per_1m_output, 99.0, "user value preserved");
        assert_eq!(e.context_window, 1_000_000);
        assert!(e.vision, "vision flag untouched");
    }

    #[test]
    fn enrich_unknown_model_is_noop() {
        let mut e = ModelRegistryEntry {
            id: "unknown-model".to_string(),
            ..Default::default()
        };
        assert!(!enrich_entry(&mut e));
        assert_eq!(e.cost_per_1m_input, 0.0);
        assert_eq!(e.context_window, 0);
    }

    #[test]
    fn every_row_has_sane_values() {
        for p in KNOWN_MODEL_PRICING {
            assert!(p.input_per_mtok_usd > 0.0, "{}", p.model_id);
            assert!(p.output_per_mtok_usd > 0.0, "{}", p.model_id);
            assert!(p.context_window > 0, "{}", p.model_id);
            assert!(
                p.cached_input_per_mtok_usd <= p.input_per_mtok_usd,
                "{} cached should not exceed input",
                p.model_id
            );
        }
    }

    // ── estimate_cost_usd (issue #4249, Phase 5 — the $0-cost turn fix) ──────

    fn approx(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-9, "expected {b}, got {a}");
    }

    #[test]
    fn estimate_prices_standard_input_and_output() {
        // opus-4-8: $5/$25 per MTok in/out. 1M in + 1M out, no cache.
        approx(
            estimate_cost_usd("claude-opus-4-8", 1_000_000, 1_000_000, 0),
            30.00,
        );
    }

    #[test]
    fn estimate_bills_cached_prefix_at_the_cheaper_rate() {
        // Fully cached input (cached == input) → cached rate only ($0.50/MTok).
        approx(
            estimate_cost_usd("claude-opus-4-8", 1_000_000, 0, 1_000_000),
            0.50,
        );
        // Half cached: 0.5M standard @ $5 + 0.5M cached @ $0.50 = 2.50 + 0.25.
        approx(
            estimate_cost_usd("claude-opus-4-8", 1_000_000, 0, 500_000),
            2.75,
        );
    }

    #[test]
    fn estimate_clamps_cached_to_input() {
        // cached_input_tokens > input_tokens must not underflow or overcharge:
        // it is clamped to input, so this is billed as fully cached.
        approx(
            estimate_cost_usd("claude-opus-4-8", 1_000_000, 0, 5_000_000),
            0.50,
        );
    }

    #[test]
    fn estimate_returns_zero_for_uncatalogued_models() {
        // "unknown, not free" — the caller treats 0.0 as no estimate available.
        assert_eq!(
            estimate_cost_usd("totally-made-up-model", 1_000_000, 1_000_000, 0),
            0.0
        );
    }

    #[test]
    fn estimate_resolves_decorated_model_ids() {
        // The catalog lookup normalizes tags/suffixes, so a decorated id
        // (e.g. the runtime "[1m]" window tag) still prices correctly.
        approx(
            estimate_cost_usd("claude-opus-4-8[1m]", 1_000_000, 0, 0),
            5.00,
        );
    }
}
