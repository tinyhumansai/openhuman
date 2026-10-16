//! The Google Places and stock-data tools against the fake integrations
//! backend, built the way the registered tool source builds them
//! ([`build_integration_tools`]).

use super::*;
use crate::integrations::test_support::spawn_fake_integration_backend;
use openhuman_embed::__host::config::Config;
use openhuman_embed::__host::security::credentials::{
    AuthService, APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME,
};
use tempfile::TempDir;
use tinytools::Tool;

fn integration_test_config(tmp: &TempDir, backend_url: &str) -> Config {
    let mut cfg = Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    cfg.api_url = Some(backend_url.to_string());
    cfg.integrations.google_places.enabled = true;
    cfg.integrations.parallel.enabled = true;
    cfg.integrations.tinyfish.enabled = true;
    cfg.integrations.stock_prices.enabled = true;
    cfg
}

fn store_test_session_token(config: &Config) {
    AuthService::from_config(config)
        .store_provider_token(
            APP_SESSION_PROVIDER,
            DEFAULT_AUTH_PROFILE_NAME,
            "test-token",
            std::collections::HashMap::new(),
            true,
        )
        .expect("store test session token");
}

fn find_tool<'a>(tools: &'a [Box<dyn Tool>], name: &str) -> &'a dyn Tool {
    tools
        .iter()
        .find(|tool| tool.name() == name)
        .map(|tool| tool.as_ref())
        .unwrap_or_else(|| panic!("tool `{name}` not registered"))
}

fn integration_tools_for_config(cfg: &Config) -> Vec<Box<dyn Tool>> {
    build_integration_tools(cfg)
}

#[test]
fn integration_families_register_when_enabled_and_signed_in() {
    let tmp = TempDir::new().unwrap();
    let mut cfg = integration_test_config(&tmp, "https://backend.example.test");
    store_test_session_token(&cfg);
    cfg.api_url = Some("https://backend.example.test".to_string());

    let tools = integration_tools_for_config(&cfg);
    let names: Vec<&str> = tools.iter().map(|tool| tool.name()).collect();
    assert_eq!(
        names,
        vec![
            "google_places_search",
            "google_places_details",
            "stock_quote",
            "stock_exchange_rate",
            "stock_options",
            "stock_crypto_series",
            "stock_commodity",
        ]
    );
}

#[test]
fn integration_families_are_absent_when_signed_out() {
    let tmp = TempDir::new().unwrap();
    let cfg = integration_test_config(&tmp, "https://backend.example.test");
    assert!(integration_tools_for_config(&cfg).is_empty());
}

#[test]
fn disabled_families_are_skipped() {
    let tmp = TempDir::new().unwrap();
    let mut cfg = integration_test_config(&tmp, "https://backend.example.test");
    cfg.integrations.stock_prices.enabled = false;
    store_test_session_token(&cfg);
    let names: Vec<String> = integration_tools_for_config(&cfg)
        .iter()
        .map(|tool| tool.name().to_string())
        .collect();
    assert_eq!(names, vec!["google_places_search", "google_places_details"]);
}

#[tokio::test]
async fn all_tools_executes_stock_family_against_fake_backend() {
    let backend = spawn_fake_integration_backend().await;
    let tmp = TempDir::new().unwrap();
    let cfg = integration_test_config(&tmp, &backend.base_url);
    store_test_session_token(&cfg);
    let tools = integration_tools_for_config(&cfg);

    let quote = find_tool(&tools, "stock_quote")
        .execute(serde_json::json!({ "symbol": "AAPL" }))
        .await
        .expect("stock_quote execute");
    assert!(quote.output().contains("AAPL"));
    assert!(quote.output().contains("latest trading day 2026-05-16"));

    let exchange = find_tool(&tools, "stock_exchange_rate")
        .execute(serde_json::json!({
            "from_currency": "BTC",
            "to_currency": "USD"
        }))
        .await
        .expect("stock_exchange_rate execute");
    assert!(exchange.output().contains("BTC/USD = 42.5"));

    let options = find_tool(&tools, "stock_options")
        .execute(serde_json::json!({
            "symbol": "AAPL",
            "require_greeks": true
        }))
        .await
        .expect("stock_options execute");
    assert!(options.output().contains("AAPL options chain"));
    assert!(options.output().contains("call 2026-06-19 @ 250"));

    let crypto = find_tool(&tools, "stock_crypto_series")
        .execute(serde_json::json!({
            "symbol": "BTC",
            "market": "USD",
            "limit": 2
        }))
        .await
        .expect("stock_crypto_series execute");
    assert!(crypto.output().contains("BTC/USD"));
    assert!(crypto.output().contains("2026-05-16"));

    let commodity = find_tool(&tools, "stock_commodity")
        .execute(serde_json::json!({
            "commodity": "WTI",
            "interval": "weekly",
            "limit": 2
        }))
        .await
        .expect("stock_commodity execute");
    assert!(commodity.output().contains("WTI (weekly)"));
    assert!(commodity.output().contains("2026-05-16  80.1000"));

    let requests = backend.requests();
    let paths: Vec<&str> = requests.iter().map(|req| req.path.as_str()).collect();
    assert_eq!(
        paths,
        vec![
            "/agent-integrations/financial-apis/quote",
            "/agent-integrations/financial-apis/exchange-rate",
            "/agent-integrations/financial-apis/options",
            "/agent-integrations/financial-apis/crypto-series",
            "/agent-integrations/financial-apis/commodity",
        ]
    );
    assert_eq!(requests[2].body["requireGreeks"], serde_json::json!(true));
}

#[tokio::test]
async fn all_tools_executes_google_places_family_against_fake_backend() {
    let backend = spawn_fake_integration_backend().await;
    let tmp = TempDir::new().unwrap();
    let cfg = integration_test_config(&tmp, &backend.base_url);
    store_test_session_token(&cfg);
    let tools = integration_tools_for_config(&cfg);

    let search = find_tool(&tools, "google_places_search")
        .execute(serde_json::json!({
            "query": "coffee",
            "max_results": 2
        }))
        .await
        .expect("google_places_search execute");
    assert!(search.output().contains("Found 2 place(s) for: coffee"));
    assert!(search.output().contains("coffee Result 1"));

    let details = find_tool(&tools, "google_places_details")
        .execute(serde_json::json!({ "place_id": "place-1-coffee" }))
        .await
        .expect("google_places_details execute");
    assert!(details.output().contains("Details for place-1-coffee"));
    assert!(details.output().contains("OPERATIONAL"));

    let requests = backend.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].body["maxResults"], serde_json::json!(2));
    assert_eq!(
        requests[1].body["placeId"],
        serde_json::json!("place-1-coffee")
    );
}
