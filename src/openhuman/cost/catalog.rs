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
pub const KNOWN_MODEL_PRICING: &[ModelPrice] = &[
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
        context_window: 1_000_000,
    },
    ModelPrice {
        provider: "openai",
        model_id: "gpt-4.1-mini",
        input_per_mtok_usd: 0.40,
        cached_input_per_mtok_usd: 0.10,
        output_per_mtok_usd: 1.60,
        context_window: 1_000_000,
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
        .filter(|p| norm.contains(p.model_id) || bare.contains(p.model_id))
        .max_by_key(|p| p.model_id.len())
}

/// Published maximum context window (tokens) for a model, if catalogued.
///
/// Convenience wrapper over [`lookup`] for callers that only need the window
/// to budget prompts / trigger compaction / pick a route. `None` ⇒ unknown.
pub fn context_window(model: &str) -> Option<u32> {
    lookup(model).map(|p| p.context_window)
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
    fn context_window_helper_resolves_known_models() {
        assert_eq!(context_window("claude-opus-4-8"), Some(1_000_000));
        assert_eq!(context_window("openai/gpt-4.1-mini"), Some(1_000_000));
        assert_eq!(context_window("deepseek-chat"), Some(128_000));
        assert_eq!(context_window("totally-made-up"), None);
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
}
