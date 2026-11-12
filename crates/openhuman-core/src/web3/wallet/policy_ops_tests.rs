use super::*;

#[test]
fn a_patch_updates_only_the_fields_it_names() {
    let mut policy = WalletPolicyConfig::default();
    apply_patch(
        &mut policy,
        serde_json::from_value(serde_json::json!({"automationEnabled": true})).unwrap(),
    )
    .unwrap();
    assert!(policy.automation_enabled);
    assert!(!policy.apply_to_interactive);
    assert!(policy.assets.is_empty());

    apply_patch(
        &mut policy,
        serde_json::from_value(serde_json::json!({
            "assets": [{"chain": "solana", "symbol": "USDC", "per_tx_max_raw": 50000000}]
        }))
        .unwrap(),
    )
    .unwrap();
    assert!(policy.automation_enabled);
    assert_eq!(policy.assets[0].per_tx_max_raw, Some(50_000_000));
}

#[test]
fn a_patch_with_an_unknown_chain_or_blank_symbol_is_rejected_and_changes_nothing() {
    let mut policy = WalletPolicyConfig::default();
    for assets in [
        serde_json::json!([{"chain": "solanaa", "symbol": "USDC"}]),
        serde_json::json!([{"chain": "solana", "symbol": " "}]),
    ] {
        let patch: WalletPolicyPatch = serde_json::from_value(
            serde_json::json!({"automationEnabled": true, "assets": assets}),
        )
        .unwrap();
        assert!(apply_patch(&mut policy, patch).is_err());
    }
    assert_eq!(policy, WalletPolicyConfig::default());
}

#[test]
fn a_patch_with_an_unknown_field_is_rejected() {
    assert!(serde_json::from_value::<WalletPolicyPatch>(serde_json::json!({"caps": 1})).is_err());
}
