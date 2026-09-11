use super::*;

fn usage(input: u64, output: u64, cached: u64, charged: f64) -> UsageInfo {
    UsageInfo {
        input_tokens: input,
        output_tokens: output,
        cached_input_tokens: cached,
        charged_amount_usd: charged,
        ..Default::default()
    }
}

#[test]
fn lookup_pricing_matches_canonical_tiers() {
    // Reasoning/agentic share the managed "Pro" rates.
    assert_eq!(lookup_pricing("reasoning-v1").input_per_mtok_usd, 0.435);
    assert_eq!(lookup_pricing("agentic-v1").output_per_mtok_usd, 0.87);
}

#[test]
fn lookup_pricing_has_a_vision_row() {
    // The vision tier must price exactly (not via the fallback) so budget
    // gating bites correctly. See PR adding the `vision-v1` tier.
    let p = lookup_pricing("vision-v1");
    assert_eq!(p.model, "vision-v1");
    assert_eq!(p.output_per_mtok_usd, 15.0);
}

#[test]
fn lookup_pricing_has_a_burst_row() {
    // The burst tier must price from its own row, not via the $3/$15
    // fallback, which would inflate worker cost and could trip budget gates.
    let p = lookup_pricing("burst-v1");
    assert_eq!(p.model, "burst-v1");
    assert_eq!(p.input_per_mtok_usd, 0.208);
    assert_eq!(p.output_per_mtok_usd, 0.208);
}

#[test]
fn lookup_pricing_falls_back_for_unknown_model() {
    let p = lookup_pricing("totally-unknown-model");
    assert_eq!(p.model, "<fallback>");
}

#[test]
fn lookup_pricing_handles_concrete_vendor_names() {
    // `claude-opus-4.7` (dotted, not a catalog id) resolves via the `opus`
    // vendor heuristic to the reasoning tier ("Pro" rates).
    assert_eq!(lookup_pricing("claude-opus-4.7").input_per_mtok_usd, 0.435);
    assert_eq!(
        lookup_pricing("claude-sonnet-4-6").output_per_mtok_usd,
        15.0
    );
}

#[test]
fn lookup_pricing_routes_coding_to_coding_row_not_agentic() {
    // Pinned per CodeRabbit feedback: when the coding-tier row
    // diverges from agentic, "coding" model strings must hit
    // PRICING_TABLE[2], not [1].
    assert_eq!(lookup_pricing("coding-v1").model, "coding-v1");
    assert_eq!(lookup_pricing("agentic-v1").model, "agentic-v1");
}

#[test]
fn estimate_call_cost_subtracts_cached_input() {
    // 1M standard input + 1M cached input + 1M output on agentic-v1 ("Pro").
    let u = usage(2_000_000, 1_000_000, 1_000_000, 0.0);
    let est = estimate_call_cost_usd("agentic-v1", &u);
    // 1M*0.435 + 1M*0.003625 + 1M*0.87 = 1.308625
    assert!((est - 1.308625).abs() < 1e-6, "got {est}");
}

#[test]
fn call_cost_prefers_charged_when_present() {
    let u = usage(100_000, 200_000, 0, 0.42);
    assert_eq!(call_cost_usd("reasoning-v1", &u), 0.42);
}

#[test]
fn call_cost_falls_back_to_estimate_when_charged_zero() {
    let u = usage(1_000_000, 0, 0, 0.0);
    // 1M input * 0.435 = 0.435
    assert!((call_cost_usd("agentic-v1", &u) - 0.435).abs() < 1e-6);
}

#[test]
fn turn_cost_accumulates_charged_and_estimated_separately() {
    let mut tc = TurnCost::new();
    tc.add_call("reasoning-v1", &usage(0, 0, 0, 0.10));
    tc.add_call("agentic-v1", &usage(1_000_000, 0, 0, 0.0)); // est: 0.435
    assert_eq!(tc.call_count, 2);
    assert!((tc.charged_usd - 0.10).abs() < 1e-6);
    assert!((tc.estimated_usd - 0.435).abs() < 1e-6);
    assert!((tc.total_usd() - 0.535).abs() < 1e-6);
}

#[test]
fn turn_cost_aggregates_token_counts() {
    let mut tc = TurnCost::new();
    tc.add_call("agentic-v1", &usage(100, 50, 20, 0.0));
    tc.add_call("agentic-v1", &usage(200, 75, 0, 0.0));
    assert_eq!(tc.input_tokens, 300);
    assert_eq!(tc.output_tokens, 125);
    assert_eq!(tc.cached_input_tokens, 20);
}
