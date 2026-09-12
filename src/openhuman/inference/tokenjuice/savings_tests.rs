use super::*;

#[test]
fn records_and_aggregates() {
    // Use a fresh local state to avoid clobbering the process-global one.
    let mut agg = SavingsAggregate::default();
    let cost = cost_saved_usd("agentic-v1", 1000);
    agg.total.add(2000, 1000, cost);
    agg.by_compressor
        .entry("smartcrusher".into())
        .or_default()
        .add(2000, 1000, cost);
    assert_eq!(agg.total.tokens_saved, 1000);
    assert!(agg.total.cost_saved_usd > 0.0);
    assert_eq!(agg.by_compressor["smartcrusher"].events, 1);
}

#[test]
fn cost_uses_input_price() {
    // agentic-v1 input pricing is used for saved-token cost estimates.
    let c = cost_saved_usd("agentic-v1", 1_000_000);
    assert!((c - 0.435).abs() < 1e-6, "got {c}");
}

#[test]
fn no_record_when_not_smaller() {
    let before = stats().total.events;
    record(ContentKind::Json, CompressorKind::SmartCrusher, 100, 100);
    record(ContentKind::Json, CompressorKind::SmartCrusher, 50, 100);
    assert_eq!(stats().total.events, before, "no-op when not smaller");
}

#[test]
fn record_saving_attributes_to_given_model() {
    // Pure aggregation on a LOCAL aggregate — no process-global state, so it
    // cannot race the other tests in this module.
    let mut agg = SavingsAggregate::default();
    agg.record_saving("turn-model-x", "smartcrusher", 2000, 1000);
    assert_eq!(agg.total.tokens_saved, 1000);
    assert!(
        agg.by_model.contains_key("turn-model-x"),
        "saving must be attributed to the supplied model"
    );
    assert!(agg.by_model["turn-model-x"].cost_saved_usd > 0.0);
}

#[tokio::test]
async fn attribution_model_falls_back_to_default_when_unscoped() {
    assert_eq!(resolve_attribution_model("default-model"), "default-model");
}

#[tokio::test]
async fn attribution_model_prefers_scoped_turn_model() {
    let got = with_turn_model("turn-model".to_string(), async {
        resolve_attribution_model("default-model")
    })
    .await;
    assert_eq!(
        got, "turn-model",
        "scoped per-turn model wins (issue #4122)"
    );
}

#[tokio::test]
async fn blank_turn_model_falls_back_to_default() {
    let got = with_turn_model("   ".to_string(), async {
        resolve_attribution_model("default-model")
    })
    .await;
    assert_eq!(got, "default-model", "blank scoped model is ignored");
}
