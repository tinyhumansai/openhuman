//! Provider charge selection at the private wire-conversion boundary.
use super::*;
use serde_json::json;
use tinyinference_llm::usage::{ChargedAmount, Usage};

fn cost(raw: Option<Value>, charged_micros: Option<i64>) -> Option<f64> {
    let mut response = ModelResponse::assistant("ok");
    response.raw = raw;
    response.usage = Some(Usage {
        charged_amount: charged_micros.map(ChargedAmount::usd_micros),
        ..Usage::default()
    });
    CompletionResponse::from_wire(response, None)
        .usage
        .unwrap()
        .cost_usd
}

#[test]
fn buyer_microcharge_overrides_relayed_cost_and_normalized_estimate() {
    assert_eq!(
        cost(
            Some(json!({"usage":{"buyer_cost_micro":3,"cost":0,"is_byok":true}})),
            Some(900_000)
        ),
        Some(0.000003)
    );
    assert_eq!(
        cost(
            Some(json!({"usage":{"buyer_cost_micro":0,"cost":0.5}})),
            Some(900_000)
        ),
        Some(0.0)
    );
}

#[test]
fn gateway_cost_precedes_typed_charge_and_typed_charge_remains_a_fallback() {
    assert_eq!(
        cost(Some(json!({"usage":{"cost":0.125}})), Some(900_000)),
        Some(0.125)
    );
    assert_eq!(cost(None, Some(12_345)), Some(0.012345));
}

#[test]
fn negative_selected_charges_stay_unknown() {
    assert_eq!(
        cost(
            Some(json!({"usage":{"buyer_cost_micro":-1,"cost":0.5}})),
            Some(900_000)
        ),
        None
    );
    assert_eq!(
        cost(Some(json!({"usage":{"cost":-0.5}})), Some(900_000)),
        None
    );
    assert_eq!(cost(None, Some(-1)), None);
}

#[test]
fn buyer_charge_survives_without_normalized_token_usage() {
    let mut response = ModelResponse::assistant("ok");
    response.raw = Some(json!({"usage":{"buyer_cost_micro":7}}));
    let usage = CompletionResponse::from_wire(response, None).usage.unwrap();
    assert_eq!(usage.cost_usd, Some(0.000007));
    assert_eq!(usage.input_tokens, 0);
}

#[test]
fn authoritative_charge_presence_prevents_invalid_billing_fallbacks() {
    for raw in [
        json!({"usage": {"buyer_cost_micro": null, "cost": 0}}),
        json!({"usage": {"buyer_cost_micro": "invalid", "cost": 0}}),
        json!({"usage": {"cost": "invalid"}}),
        json!({"openhuman_usage_meta": {"charged_amount_usd": null}, "usage": {"cost": 0}}),
    ] {
        assert_eq!(cost(Some(raw), Some(12)), None);
    }
    assert_eq!(
        cost(
            Some(
                json!({"openhuman_usage_meta": {"charged_amount_usd": 0.0000064}, "usage": {"buyer_cost_micro": 42, "cost": 1}})
            ),
            Some(12)
        ),
        Some(0.0000064)
    );
}
