//! Tests for the integrations module root (types + client construction).
//!
//! Split out of `mod.rs` to keep the module root export-focused. Declared via
//! `#[path = "mod_tests.rs"] mod tests;` so `super::*` still resolves to the
//! `integrations` module.

use super::*;

#[test]
fn tool_scope_equality() {
    assert_eq!(ToolScope::All, ToolScope::All);
    assert_ne!(ToolScope::All, ToolScope::CliRpcOnly);
    assert_ne!(ToolScope::AgentOnly, ToolScope::CliRpcOnly);
}

#[test]
fn backend_response_deserializes() {
    let json = r#"{"success": true, "data": {"foo": 42}}"#;
    let resp: BackendResponse<serde_json::Value> = serde_json::from_str(json).unwrap();
    assert!(resp.success);
    assert_eq!(resp.data.unwrap()["foo"], 42);
}

#[test]
fn backend_response_without_data() {
    let json = r#"{"success": true}"#;
    let resp: BackendResponse<serde_json::Value> = serde_json::from_str(json).unwrap();
    assert!(resp.success);
    assert!(resp.data.is_none());
}

#[test]
fn integration_pricing_defaults_on_missing_fields() {
    let json = r#"{"integrations": {}}"#;
    let pricing: IntegrationPricing = serde_json::from_str(json).unwrap();
    assert!(pricing.integrations.apify.is_none());
    assert!(pricing.integrations.twilio.is_none());
    assert!(pricing.integrations.google_places.is_none());
    assert!(pricing.integrations.parallel.is_none());
    assert!(pricing.integrations.tinyfish.is_none());
}

#[test]
fn build_client_returns_none_when_no_auth_token() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config = crate::openhuman::config::Config {
        workspace_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..crate::openhuman::config::Config::default()
    };
    assert!(build_client(&config).is_none());
}
