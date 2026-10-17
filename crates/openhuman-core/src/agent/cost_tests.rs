use super::*;
use tinyagents_harness::cost::{CostSource, CostTally};

fn counts(input: u64, output: u64, cached: u64) -> BilledUsage {
    BilledUsage::from_counts(input, output).with_cached_input_tokens(cached)
}

/// Fold one call into `turn` the way the live stop-hook chain does: as a
/// harness `ModelResponse` (the host's wire form, charge in `raw`) recorded by
/// the harness `StopHookMetrics` impl, priced through the catalog lookup.
fn record(turn: &mut tinyagents_harness::cost::TurnCost, model: &str, usage: &BilledUsage) {
    use tinyagents_harness::middleware::StopHookMetrics as _;
    let response = crate::agent::tinyagents::model::native_model_response_for_request(
        &crate::inference::provider::ChatResponse {
            text: Some("ok".into()),
            usage: Some(*usage),
            ..Default::default()
        },
        &tinyinference_llm::model::ModelRequest::default(),
    );
    turn.record_model(model, &response);
}

/// The thread that exposed the guess: glm-5.3-flash is not catalogued, and a
/// $3 / $0.30 / $15 per-MTok default priced its 72-call turn at $3.02 when the
/// provider billed about $0.22.
const UNCATALOGUED: &str = "openrouter/z-ai/glm-5.3-flash";
const CATALOGUED: &str = "claude-sonnet-4-6";

#[test]
fn an_uncatalogued_model_has_no_price_and_no_estimate() {
    assert!(lookup_pricing(UNCATALOGUED).is_none());
    let usage = counts(312_065 + 5_398_592, 30_790, 5_398_592);
    assert_eq!(estimate_call_cost_usd(UNCATALOGUED, &usage), None);
    assert_eq!(call_cost(UNCATALOGUED, &usage), CallCost::Unknown);
}

#[test]
fn a_catalogued_model_is_estimated_at_its_list_rates() {
    let pricing = lookup_pricing(CATALOGUED).expect("catalogued");
    assert_eq!(pricing.input_per_token, Some(3.0 / 1_000_000.0));
    // 1M uncached input at $3 + 1M cached at $0.30 + 1M output at $15.
    let usage = counts(2_000_000, 1_000_000, 1_000_000);
    let estimate = estimate_call_cost_usd(CATALOGUED, &usage).expect("priced");
    assert!((estimate - 18.30).abs() < 1e-9, "estimate {estimate}");
    assert_eq!(call_cost(CATALOGUED, &usage), CallCost::Estimated(estimate));
}

#[test]
fn a_reported_charge_wins_even_when_it_is_zero() {
    let charged = counts(1_000, 100, 0).with_charged_usd(0.0123);
    assert_eq!(call_cost(UNCATALOGUED, &charged), CallCost::Charged(0.0123));
    // A free route reports $0: that is a known cost, not a missing one.
    let free = counts(1_000, 100, 0).with_charged_usd(0.0);
    assert_eq!(call_cost(UNCATALOGUED, &free), CallCost::Charged(0.0));
    // `with_reported_charge(None)` leaves the cost unknown.
    let none = counts(1_000, 100, 0).with_reported_charge(None);
    assert_eq!(call_cost(UNCATALOGUED, &none), CallCost::Unknown);
}

#[test]
fn an_estimate_carried_on_the_usage_is_not_taken_for_a_charge() {
    let estimated = counts(1_000_000, 0, 0).with_estimated_usd(9.99);
    assert_eq!(
        call_cost(CATALOGUED, &estimated),
        CallCost::Estimated(3.0),
        "a carried estimate is re-priced from the catalog, not trusted as billed"
    );
}

#[test]
fn a_turn_with_any_unknown_call_has_no_total() {
    let mut turn = turn_cost();
    record(
        &mut turn,
        UNCATALOGUED,
        &counts(10, 1, 0).with_charged_usd(0.5),
    );
    assert_eq!(turn.total_usd(), Some(0.5));
    assert_eq!(turn.cost.source, CostSource::Charged);

    record(&mut turn, CATALOGUED, &counts(1_000_000, 0, 0));
    assert_eq!(turn.cost.source, CostSource::Estimated);
    assert!((turn.total_usd().unwrap() - 3.5).abs() < 1e-9);

    record(&mut turn, UNCATALOGUED, &counts(10, 1, 0));
    assert_eq!(turn.cost.source, CostSource::Unknown);
    assert_eq!(turn.total_usd(), None, "an unknown call voids the total");
    assert_eq!(turn.call_count, 3);
    assert_eq!(turn.input_tokens, 1_000_020);
}

#[test]
fn merged_tallies_keep_the_least_certain_source() {
    let mut parent = CostTally::default();
    parent.add(CallCost::Charged(1.0));
    let mut child = CostTally::default();
    child.add(CallCost::Estimated(0.25));
    parent.merge(child);
    assert_eq!(parent.source, CostSource::Estimated);
    assert_eq!(parent.usd(), Some(1.25));

    let mut unknown = CostTally::default();
    unknown.add(CallCost::Unknown);
    parent.merge(unknown);
    assert_eq!(parent.usd(), None);
}

#[test]
fn free_models_are_priced_at_zero() {
    for model in [
        "deepseek/deepseek-chat-v3.1:free",
        "openrouter/meta-llama/llama-3.3-70b-instruct:free",
        "Totally-Unknown-Model:FREE",
    ] {
        assert!(is_free_model(model), "{model}");
        let usage = counts(1_000_000, 1_000_000, 0);
        assert_eq!(estimate_call_cost_usd(model, &usage), Some(0.0), "{model}");
        assert_eq!(call_cost(model, &usage), CallCost::Estimated(0.0));
    }
    assert!(!is_free_model("deepseek/deepseek-chat"));
}

#[test]
fn unknown_model_has_no_known_price() {
    assert!(lookup_pricing("totally-unknown-model").is_none());
    assert_eq!(
        estimate_call_cost_usd("totally-unknown-model", &counts(1_000, 100, 0)),
        None
    );
}

/// Regression pin for the move of the cost vocabulary into the harness: a
/// scripted four-call turn (two charged, one catalogue-estimated, one free
/// route) totals exactly what it did before the cutover.
#[test]
fn a_scripted_multi_call_turn_totals_are_unchanged() {
    let calls = [
        (
            UNCATALOGUED,
            counts(4_000, 200, 3_000).with_charged_usd(0.0125),
        ),
        (
            UNCATALOGUED,
            counts(4_500, 150, 4_000).with_charged_usd(0.0075),
        ),
        // 1M uncached input at $3 + 1M cached at $0.30 + 1M output at $15.
        (CATALOGUED, counts(2_000_000, 1_000_000, 1_000_000)),
        ("vendor/model:free", counts(900, 50, 0)),
    ];
    let mut turn = turn_cost();
    for (model, usage) in &calls {
        record(&mut turn, model, usage);
    }
    assert_eq!(turn.call_count, 4);
    assert_eq!(turn.input_tokens, 2_009_400);
    assert_eq!(turn.output_tokens, 1_000_400);
    assert_eq!(turn.cached_input_tokens, 1_007_000);
    assert_eq!(turn.cost.source, CostSource::Estimated);
    let total = turn.total_usd().expect("every call is priced");
    assert!(
        (total - (0.0125 + 0.0075 + 18.30)).abs() < 1e-9,
        "total {total}"
    );
}
