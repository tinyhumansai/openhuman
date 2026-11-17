use super::*;
use tinyinference_llm::usage::BilledUsage;

/// The flow `agent`/`llm` node result exposes usage under these exact keys;
/// they are a contract with saved flows, so pin the literal JSON.
#[test]
fn usage_to_json_wire_shape_is_stable() {
    assert_eq!(usage_to_json(&None), serde_json::Value::Null);
    let usage = BilledUsage::from_counts(100, 20)
        .with_context_window(128_000)
        .with_cached_input_tokens(40)
        .with_cache_creation_tokens(10)
        .with_reasoning_tokens(7)
        .with_charged_usd(0.0123);
    assert_eq!(
        usage_to_json(&Some(usage)),
        serde_json::json!({
            "input_tokens": 100,
            "output_tokens": 20,
            "context_window": 128000,
            "cached_input_tokens": 40,
            "cache_creation_tokens": 10,
            "reasoning_tokens": 7,
            "charged_amount_usd": 0.0123
        })
    );
}
