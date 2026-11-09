use super::*;

#[test]
fn input_price_is_per_million_tokens() {
    let p = input_per_mtok_usd("claude-sonnet-4-6").expect("catalogued model");
    assert!((p - 3.0).abs() < 1e-6, "got {p}");
}

#[test]
fn unpriced_model_has_no_price() {
    assert_eq!(input_per_mtok_usd("no-such-model-xyz"), None);
}

#[test]
fn no_request_when_not_smaller() {
    assert!(request(CompressorKind::SmartCrusher, 100, 100).is_none());
    assert!(request(CompressorKind::SmartCrusher, 50, 100).is_none());
}

#[test]
fn request_carries_attribution_and_token_counts() {
    assert!(request(CompressorKind::SmartCrusher, 2000, 1000).is_none());
    let req = request_for(
        "test-model".to_string(),
        CompressorKind::SmartCrusher,
        2000,
        1000,
    )
    .unwrap();
    assert_eq!(req.model, "test-model");
    assert_eq!(req.compressor, CompressorKind::SmartCrusher.as_str());
    assert_eq!((req.original_tokens, req.compacted_tokens), (2000, 1000));
}

/// Reference totals computed with the pre-module host ledger's formula
/// (`tokens_saved * input_per_token` per event, summed). The module now owns
/// the aggregation (`tinyjuice::savings::SavingsLedger::record`: `saved /
/// 1e6 * input_per_mtok_usd`); the host only supplies the price, so folding
/// host-built requests through the module's documented math must reproduce
/// these totals.
#[test]
fn host_pricing_over_module_math_reproduces_the_old_totals() {
    use std::collections::HashMap;

    // (model, compressor, original, compacted)
    let events: [(&str, CompressorKind, u64, u64); 5] = [
        (
            "claude-sonnet-4-6",
            CompressorKind::SmartCrusher,
            2000,
            1000,
        ),
        (
            "claude-sonnet-4-6",
            CompressorKind::SmartCrusher,
            10000,
            2500,
        ),
        ("claude-sonnet-4-6", CompressorKind::TextCrusher, 5000, 5000),
        ("no-such-model-xyz", CompressorKind::TextCrusher, 4000, 1000),
        ("claude-sonnet-4-6", CompressorKind::TextCrusher, 800, 900),
    ];

    // key -> (events, original, compacted, saved, cost)
    let mut total = (0u64, 0u64, 0u64, 0u64, 0f64);
    let mut by_model: HashMap<String, (u64, u64, f64)> = HashMap::new();
    let mut by_compressor: HashMap<String, (u64, u64, f64)> = HashMap::new();
    for (model, compressor, original, compacted) in events {
        let Some(req) = request_for(model.to_string(), compressor, original, compacted) else {
            continue;
        };
        // The module's per-event math over the host-supplied price.
        let saved = req.original_tokens - req.compacted_tokens;
        let cost = req
            .input_per_mtok_usd
            .map_or(0.0, |price| saved as f64 / 1_000_000.0 * price);
        total.0 += 1;
        total.1 += req.original_tokens;
        total.2 += req.compacted_tokens;
        total.3 += saved;
        total.4 += cost;
        let m = by_model.entry(req.model.clone()).or_default();
        *m = (m.0 + 1, m.1 + saved, m.2 + cost);
        let c = by_compressor.entry(req.compressor.clone()).or_default();
        *c = (c.0 + 1, c.1 + saved, c.2 + cost);
    }

    // Old totals: 3 shrinking events (equal and expanded are ignored).
    assert_eq!(
        (total.0, total.1, total.2, total.3),
        (3, 16000, 4500, 11500)
    );
    // (1000 + 7500) tokens priced at $3 / 1M; the unpriced model adds none.
    assert!((total.4 - 0.0255).abs() < 1e-12, "cost {}", total.4);

    let sonnet = by_model["claude-sonnet-4-6"];
    assert_eq!((sonnet.0, sonnet.1), (2, 8500));
    assert!((sonnet.2 - 0.0255).abs() < 1e-12);
    assert_eq!(by_model["no-such-model-xyz"], (1, 3000, 0.0));

    let smart = by_compressor[CompressorKind::SmartCrusher.as_str()];
    assert_eq!((smart.0, smart.1), (2, 8500));
    assert!((smart.2 - 0.0255).abs() < 1e-12);
    assert_eq!(
        by_compressor[CompressorKind::TextCrusher.as_str()],
        (1, 3000, 0.0)
    );
}
