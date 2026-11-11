use super::*;

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
fn tinyagents_snapshot_contains_all_known_rows() {
    let snapshot = tinyagents_catalog_snapshot();
    assert_eq!(snapshot.schema_version, 1);
    assert_eq!(snapshot.currency, "USD");
    assert_eq!(snapshot.unit, "token");
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
fn enrich_backfills_minimax_vision() {
    let mut entry = ModelRegistryEntry {
        id: "minimax/minimax-m3".to_string(),
        ..Default::default()
    };
    assert!(enrich_entry(&mut entry));
    assert!(entry.vision);
}
