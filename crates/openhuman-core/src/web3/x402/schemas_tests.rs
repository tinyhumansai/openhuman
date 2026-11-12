use super::*;

#[test]
fn summary_response_preserves_legacy_camel_case_rpc_shape() {
    let value = summary_value(tinywallet_bus::wire::X402DetailedSpendingSummary {
        session_total_atomic: 11,
        daily_total_atomic: 12,
        monthly_total_atomic: 13,
        session_count: 2,
        daily_count: 3,
        monthly_count: 4,
        reserved_atomic: 99,
        per_request_max_atomic: 100,
        daily_max_atomic: 200,
        monthly_max_atomic: 300,
    });

    assert_eq!(
        value,
        json!({
            "summary": {
                "sessionTotalAtomic": 11,
                "dailyTotalAtomic": 12,
                "monthlyTotalAtomic": 13,
                "sessionCount": 2,
                "dailyCount": 3,
                "monthlyCount": 4,
            },
            "budget": {
                "perRequestMaxAtomic": 100,
                "dailyMaxAtomic": 200,
                "monthlyMaxAtomic": 300,
            },
        })
    );
}

#[test]
fn budget_response_preserves_legacy_camel_case_rpc_shape() {
    assert_eq!(
        budget_value(1, 20, 300),
        json!({
            "budget": {
                "perRequestMaxAtomic": 1,
                "dailyMaxAtomic": 20,
                "monthlyMaxAtomic": 300,
            }
        })
    );
}

#[test]
fn payment_list_preserves_legacy_rpc_shape_and_rfc3339_values() {
    let value = payments_value(vec![tinywallet_bus::wire::X402PaymentRecord {
        id: "payment-1".into(),
        url: "https://api.example.test/resource".into(),
        asset: "USDC".into(),
        amount_atomic: 2_500,
        amount_display: "0.002500 USDC".into(),
        recipient: "0x0000000000000000000000000000000000000001".into(),
        network: "eip155:1".into(),
        tx_signature: Some("0xabc".into()),
        status: tinywallet_bus::wire::X402PaymentStatus::Settled,
        timestamp: "2026-10-11T19:02:03Z".into(),
        authorization_valid_until: Some("2026-10-11T19:03:03Z".into()),
        solana_last_valid_block_height: None,
        authorization_expired_at: None,
        authorization_expiry_observed_height: None,
        session_id: "session-1".into(),
        thread_id: Some("thread-1".into()),
    }]);

    assert_eq!(
        value,
        json!({
            "payments": [{
                "id": "payment-1",
                "url": "https://api.example.test/resource",
                "asset": "USDC",
                "amountAtomic": 2500,
                "amountDisplay": "0.002500 USDC",
                "recipient": "0x0000000000000000000000000000000000000001",
                "network": "eip155:1",
                "txSignature": "0xabc",
                "status": "settled",
                "timestamp": "2026-10-11T19:02:03Z",
                "authorizationValidUntil": "2026-10-11T19:03:03Z",
                "sessionId": "session-1",
                "threadId": "thread-1"
            }]
        })
    );
}

#[test]
fn fetch_controller_is_registered_with_its_schema() {
    assert!(all_controller_schemas()
        .iter()
        .any(|schema| schema.namespace == "x402" && schema.function == "fetch"));
    let registered = all_registered_controllers();
    let fetch = registered
        .iter()
        .find(|controller| controller.schema.function == "fetch")
        .expect("x402 fetch controller is registered");
    let required: Vec<&str> = fetch
        .schema
        .inputs
        .iter()
        .filter(|field| field.required)
        .map(|field| field.name)
        .collect();
    assert_eq!(required, vec!["url", "maxAmountAtomic"]);
}

fn update_budget_params(value: Value) -> Map<String, Value> {
    value.as_object().cloned().expect("object params")
}

#[test]
fn update_budget_reads_every_input_the_schema_declares() {
    let schema = schemas("update_budget");
    let mut params = Map::new();
    for (index, input) in schema.inputs.iter().enumerate() {
        params.insert(input.name.to_string(), json!(100 * (index as u64 + 1)));
    }
    assert_eq!(params.len(), 3, "schema declares three limits");

    let parsed = parse_update_budget(params).expect("schema param names parse");
    assert_eq!(parsed.per_request_max, Some(100));
    assert_eq!(parsed.daily_max, Some(200));
    assert_eq!(parsed.monthly_max, Some(300));
}

#[test]
fn update_budget_still_accepts_snake_case_limit_names() {
    let parsed = parse_update_budget(update_budget_params(json!({
        "per_request_max": 1,
        "daily_max": 2,
        "monthly_max": 3,
    })))
    .expect("snake_case names parse");
    assert_eq!(
        (parsed.per_request_max, parsed.daily_max, parsed.monthly_max),
        (Some(1), Some(2), Some(3))
    );
}
