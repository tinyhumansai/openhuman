use super::*;

#[test]
fn default_policy_keeps_interactive_uncapped_and_automation_off() {
    let policy = WalletPolicyConfig::default();
    assert!(!policy.automation_enabled);
    assert!(!policy.apply_to_interactive);
    assert!(policy.assets.is_empty());
}

#[test]
fn sol_and_usdc_caps_parse_from_the_config_table() {
    let config: crate::config::schema::Config = toml::from_str(
        r#"
[wallet_policy]
automation_enabled = true

[[wallet_policy.assets]]
chain = "solana"
symbol = "SOL"
per_tx_max_raw = 333333333
daily_max_raw = 666666666
recipient_weekly_max_raw = 333333333

[[wallet_policy.assets]]
chain = "solana"
symbol = "USDC"
per_tx_max_raw = 50000000
daily_max_raw = 100000000
recipient_weekly_max_raw = 50000000
"#,
    )
    .unwrap();
    let policy = config.wallet_policy;
    assert!(policy.automation_enabled);
    assert_eq!(policy.assets.len(), 2);
    assert_eq!(policy.assets[1].symbol, "USDC");
    assert_eq!(policy.assets[1].per_tx_max_raw, Some(50_000_000));
    assert_eq!(policy.assets[1].daily_max_raw, Some(100_000_000));
    assert_eq!(policy.assets[1].recipient_weekly_max_raw, Some(50_000_000));
    assert_eq!(policy.assets[0].token_address, None);
}

#[derive(Default)]
struct MapEnv(std::collections::HashMap<String, String>);

impl MapEnv {
    fn with(mut self, key: &str, value: &str) -> Self {
        self.0.insert(key.to_string(), value.to_string());
        self
    }
}

impl crate::config::schema::load::env::EnvLookup for MapEnv {
    fn get(&self, key: &str) -> Option<String> {
        self.0.get(key).cloned()
    }
}

#[test]
fn env_overlay_wallet_policy_sets_flags_and_assets_and_ignores_garbage() {
    let mut cfg = crate::config::schema::Config::default();
    cfg.apply_env_overlay_with(
        &MapEnv::default()
            .with("OPENHUMAN_WALLET_POLICY_AUTOMATION_ENABLED", "true")
            .with("OPENHUMAN_WALLET_POLICY_APPLY_TO_INTERACTIVE", "1")
            .with(
                "OPENHUMAN_WALLET_POLICY_ASSETS",
                r#"[{"chain":"solana","symbol":"USDC","per_tx_max_raw":50000000}]"#,
            ),
    );
    assert!(cfg.wallet_policy.automation_enabled);
    assert!(cfg.wallet_policy.apply_to_interactive);
    assert_eq!(cfg.wallet_policy.assets.len(), 1);
    assert_eq!(cfg.wallet_policy.assets[0].per_tx_max_raw, Some(50_000_000));

    // A malformed asset list keeps the configured caps instead of clearing them.
    cfg.apply_env_overlay_with(
        &MapEnv::default()
            .with("OPENHUMAN_WALLET_POLICY_ASSETS", "not json")
            .with("OPENHUMAN_WALLET_POLICY_AUTOMATION_ENABLED", "maybe"),
    );
    assert!(cfg.wallet_policy.automation_enabled);
    assert_eq!(cfg.wallet_policy.assets.len(), 1);
}

#[test]
fn a_misspelled_asset_cap_field_is_rejected_instead_of_leaving_the_cap_off() {
    let error = serde_json::from_str::<Vec<WalletAssetCap>>(
        r#"[{"chain":"solana","symbol":"USDC","daily_max_raww":100000000}]"#,
    )
    .unwrap_err();
    assert!(error.to_string().contains("daily_max_raww"), "{error}");
}
