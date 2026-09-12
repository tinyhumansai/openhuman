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

#[test]
fn minimax_rows_and_vision_normalization() {
    assert_eq!(
        lookup("minimax/minimax-m3").unwrap().context_window,
        1_000_000
    );
    assert_eq!(
        lookup("minimax-m2.7-highspeed")
            .unwrap()
            .output_per_mtok_usd,
        2.40
    );
    assert!(model_accepts_image_input("MiniMax/minimax-m3:latest"));
    assert!(default_registry_entries()
        .iter()
        .any(|entry| entry.id == "minimax-m3" && entry.vision));
}

#[test]
fn minimax_m3_uses_standard_tier_through_512k() {
    approx(estimate_cost_usd("minimax-m3", 512_000, 0, 0), 0.1536);
    approx(estimate_cost_usd("minimax-m3", 512_001, 0, 0), 0.3072006);
}

#[test]
fn enrich_backfills_minimax_vision() {
    let mut entry = ModelRegistryEntry {
        id: "minimax/minimax-m3".to_string(),
        ..Default::default()
    };
    assert!(enrich_entry(&mut entry));
    assert!(entry.vision);
}
