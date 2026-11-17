use super::*;
use tinyagents_harness::cost::CostSource;
use tinyagents_session::transcript::spend::CostSplit;
use tinyagents_session::transcript::UsageCostSource;

const UNCATALOGUED: &str = "openrouter/z-ai/glm-5.3-flash";
const CATALOGUED: &str = "claude-sonnet-4-6";

/// The "RSI framework spec" thread: every turn was written before costs
/// stated their source, at a guessed $3 / $0.30 / $15 rate, $4.25 in all for
/// about $0.30 billed. With no list price for the model, the thread's cost is
/// not known, and is not reported.
#[test]
fn legacy_turns_on_an_uncatalogued_model_report_no_cost() {
    let split = CostSplit {
        unpriced_turns: 3,
        unpriced_input_tokens: 7_021_942,
        unpriced_output_tokens: 42_669,
        unpriced_cached_input_tokens: 6_466_688,
        ..CostSplit::default()
    };
    let cost = recorded_cost(&split, Some(UNCATALOGUED));
    assert_eq!(cost.usd(), None);
    assert_eq!(cost.source, CostSource::Unknown);
}

/// Recorded charges are summed as recorded, never re-priced at a rate.
#[test]
fn recorded_charges_are_reported_as_charged() {
    let split = CostSplit {
        priced_cost_usd: 0.2986,
        priced_source: Some(UsageCostSource::Charged),
        ..CostSplit::default()
    };
    let cost = recorded_cost(&split, Some(UNCATALOGUED));
    assert_eq!(cost.usd(), Some(0.2986));
    assert_eq!(cost.source, CostSource::Charged);
}

/// Legacy turns on a model with a list price are re-priced from the catalog
/// and the total is marked as an estimate.
#[test]
fn legacy_turns_on_a_catalogued_model_are_estimated() {
    let split = CostSplit {
        priced_cost_usd: 1.0,
        priced_source: Some(UsageCostSource::Charged),
        unpriced_turns: 1,
        unpriced_input_tokens: 1_000_000,
        unpriced_output_tokens: 0,
        unpriced_cached_input_tokens: 0,
    };
    let cost = recorded_cost(&split, Some(CATALOGUED));
    assert_eq!(cost.source, CostSource::Estimated);
    assert!((cost.usd().unwrap() - 4.0).abs() < 1e-9);
}

/// A thread whose last recorded turn had an unknown cost stays unknown.
#[test]
fn a_recorded_unknown_turn_without_a_catalog_price_stays_unknown() {
    let split = CostSplit {
        priced_cost_usd: 0.5,
        priced_source: Some(UsageCostSource::Charged),
        unpriced_turns: 1,
        unpriced_input_tokens: 10,
        unpriced_output_tokens: 1,
        unpriced_cached_input_tokens: 0,
    };
    assert_eq!(recorded_cost(&split, Some(UNCATALOGUED)).usd(), None);
    assert_eq!(recorded_cost(&split, None).usd(), None);
}
