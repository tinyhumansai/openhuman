use super::*;

#[test]
fn cost_config_defaults() {
    let c = CostConfig::default();
    assert!(c.enabled);
    assert_eq!(c.monthly_limit_usd, 100.0);
    assert!(c.prices.is_empty());
    assert!(c.dashboard.enabled);
    assert_eq!(c.dashboard.currency, "USD");
    assert!((c.dashboard.warn_threshold - 0.8).abs() < f64::EPSILON);
    assert!((c.dashboard.alert_threshold - 0.95).abs() < f64::EPSILON);
}

#[test]
fn cost_dashboard_config_serde_roundtrip() {
    let toml = r#"
        enabled = true
        [dashboard]
        enabled = false
        currency = "EUR"
        warn_threshold = 0.5
        alert_threshold = 0.9
    "#;
    let c: CostConfig = toml::from_str(toml).unwrap();
    assert!(!c.dashboard.enabled);
    assert_eq!(c.dashboard.currency, "EUR");
    assert!((c.dashboard.warn_threshold - 0.5).abs() < f64::EPSILON);
    assert!((c.dashboard.alert_threshold - 0.9).abs() < f64::EPSILON);
}

#[test]
fn cost_config_serde_roundtrip() {
    let c = CostConfig::default();
    let json = serde_json::to_string(&c).unwrap();
    let back: CostConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(back.monthly_limit_usd, 100.0);
}

#[test]
fn cost_config_toml_with_custom_values() {
    let toml = r#"
        enabled = true
        monthly_limit_usd = 500.0
    "#;
    let c: CostConfig = toml::from_str(toml).unwrap();
    assert!(c.enabled);
    assert_eq!(c.monthly_limit_usd, 500.0);
}

/// Every install that ran a build with the daily cap still has
/// `daily_limit_usd` in its `config.toml`. Retiring the field must not make
/// those files unparseable: `CostConfig` does not `deny_unknown_fields`, so the
/// key is accepted and ignored. If that ever changes, every such user fails to
/// load their config on upgrade — a far worse bug than the cap was.
#[test]
fn retired_daily_limit_key_is_accepted_and_ignored() {
    let toml = r#"
        enabled = true
        daily_limit_usd = 10.0
        monthly_limit_usd = 500.0
    "#;
    let c: CostConfig =
        toml::from_str(toml).expect("a config carrying the retired daily key must still parse");
    assert!(c.enabled);
    assert_eq!(c.monthly_limit_usd, 500.0);
}

#[test]
fn model_pricing_defaults_to_zero() {
    let p: ModelPricing = serde_json::from_str("{}").unwrap();
    assert_eq!(p.input, 0.0);
    assert_eq!(p.output, 0.0);
}
