use super::*;

#[test]
fn legacy_subagent_usage_keeps_its_wire_shape_and_does_not_claim_a_charge() {
    let raw = serde_json::json!({
        "input_tokens": 10, "output_tokens": 5,
        "cached_input_tokens": 1, "charged_amount_usd": 0.5,
    });
    let mut usage: SubagentUsage = serde_json::from_value(raw.clone()).unwrap();
    assert_eq!(
        usage.cost_source,
        tinyagents_harness::cost::CostSource::Unknown
    );
    assert_eq!(usage.cost().usd(), None);
    assert_eq!(serde_json::to_value(usage).unwrap(), raw);
    usage.cost_source = tinyagents_harness::cost::CostSource::Charged;
    let charged = serde_json::to_value(usage).unwrap();
    assert_eq!(charged["cost_source"], "charged");
    assert_eq!(
        serde_json::from_value::<SubagentUsage>(charged)
            .unwrap()
            .cost()
            .usd(),
        Some(0.5)
    );
}
