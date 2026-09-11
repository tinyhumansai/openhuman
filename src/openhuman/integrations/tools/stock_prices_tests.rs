use super::*;
use crate::openhuman::integrations::ToolScope;

fn test_client() -> Arc<IntegrationClient> {
    Arc::new(IntegrationClient::new("http://test".into(), "tok".into()))
}

#[test]
fn quote_tool_metadata() {
    let t = StockQuoteTool::new(test_client());
    assert_eq!(t.name(), "stock_quote");
    assert_eq!(t.scope(), ToolScope::All);
    assert!(t.description().to_lowercase().contains("stock"));
}

#[test]
fn exchange_rate_tool_metadata() {
    let t = StockExchangeRateTool::new(test_client());
    assert_eq!(t.name(), "stock_exchange_rate");
    let schema = t.parameters_schema();
    let req = schema["required"].as_array().unwrap();
    assert!(req.iter().any(|v| v == "from_currency"));
    assert!(req.iter().any(|v| v == "to_currency"));
}

#[test]
fn options_tool_metadata() {
    let t = StockOptionsTool::new(test_client());
    assert_eq!(t.name(), "stock_options");
}

#[test]
fn crypto_series_tool_metadata() {
    let t = StockCryptoSeriesTool::new(test_client());
    assert_eq!(t.name(), "stock_crypto_series");
}

#[test]
fn commodity_tool_metadata() {
    let t = StockCommodityTool::new(test_client());
    assert_eq!(t.name(), "stock_commodity");
}

#[tokio::test]
async fn quote_rejects_missing_symbol() {
    let t = StockQuoteTool::new(test_client());
    assert!(t.execute(json!({})).await.is_err());
}

#[tokio::test]
async fn exchange_rate_rejects_missing_currency() {
    let t = StockExchangeRateTool::new(test_client());
    assert!(t.execute(json!({"from_currency": "BTC"})).await.is_err());
}

#[test]
fn quote_response_deserializes() {
    let json = r#"{
        "quote": {
            "symbol": "AAPL",
            "price": 271.06,
            "open": 270.0,
            "high": 272.5,
            "low": 269.5,
            "volume": 1000000,
            "previousClose": 268.5,
            "change": 2.56,
            "changePercent": "0.95%",
            "latestTradingDay": "2026-04-23"
        },
        "costUsd": 0.001
    }"#;
    let resp: QuoteResponse = serde_json::from_str(json).unwrap();
    assert_eq!(resp.quote.symbol, "AAPL");
    assert!((resp.quote.price - 271.06).abs() < 1e-6);
}

#[test]
fn exchange_rate_response_deserializes() {
    let json = r#"{
        "rate": {
            "fromCurrency": "BTC",
            "toCurrency": "USD",
            "rate": 77421.13,
            "bid": 77418.0,
            "ask": 77424.26,
            "lastRefreshed": "2026-04-23 10:00:00",
            "timeZone": "UTC"
        },
        "costUsd": 0.001
    }"#;
    let resp: ExchangeRateResponse = serde_json::from_str(json).unwrap();
    assert_eq!(resp.rate.from_currency, "BTC");
    assert!((resp.rate.rate - 77421.13).abs() < 1e-6);
}
