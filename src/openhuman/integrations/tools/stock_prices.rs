//! Stock-price / market-data integration tools (backed by Alpha Vantage on the backend).
//!
//! **Scope**: All (agent loop + CLI/RPC).
//!
//! **Endpoints** (mounted under `/agent-integrations/financial-apis/*` on the backend,
//! which proxies Alpha Vantage):
//!   - `POST /quote`          — `GLOBAL_QUOTE` for stocks and indices
//!   - `POST /options`        — `REALTIME_OPTIONS` (optional greeks)
//!   - `POST /exchange-rate`  — `CURRENCY_EXCHANGE_RATE` (FX and crypto, e.g. BTC/USD)
//!   - `POST /crypto-series`  — `DIGITAL_CURRENCY_DAILY` OHLCV
//!   - `POST /commodity`      — futures: WTI / BRENT / NATURAL_GAS
//!
//! Pricing is metered by the backend; the response includes `costUsd` per call.

use super::IntegrationClient;
use crate::openhuman::tools::traits::{Tool, ToolResult};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

const PATH_QUOTE: &str = "/agent-integrations/financial-apis/quote";
const PATH_OPTIONS: &str = "/agent-integrations/financial-apis/options";
const PATH_EXCHANGE_RATE: &str = "/agent-integrations/financial-apis/exchange-rate";
const PATH_CRYPTO_SERIES: &str = "/agent-integrations/financial-apis/crypto-series";
const PATH_COMMODITY: &str = "/agent-integrations/financial-apis/commodity";

// ── Response types ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct QuoteResponse {
    quote: Quote,
    #[serde(rename = "costUsd", default)]
    cost_usd: f64,
}

#[derive(Debug, Deserialize)]
struct Quote {
    symbol: String,
    price: f64,
    open: f64,
    high: f64,
    low: f64,
    volume: f64,
    #[serde(rename = "previousClose")]
    previous_close: f64,
    change: f64,
    #[serde(rename = "changePercent", default)]
    change_percent: String,
    #[serde(rename = "latestTradingDay", default)]
    latest_trading_day: String,
}

#[derive(Debug, Deserialize)]
struct ExchangeRateResponse {
    rate: ExchangeRate,
    #[serde(rename = "costUsd", default)]
    cost_usd: f64,
}

#[derive(Debug, Deserialize)]
struct ExchangeRate {
    #[serde(rename = "fromCurrency")]
    from_currency: String,
    #[serde(rename = "toCurrency")]
    to_currency: String,
    rate: f64,
    #[serde(default)]
    bid: Option<f64>,
    #[serde(default)]
    ask: Option<f64>,
    #[serde(rename = "lastRefreshed", default)]
    last_refreshed: String,
    #[serde(rename = "timeZone", default)]
    time_zone: String,
}

#[derive(Debug, Deserialize)]
struct OptionsResponse {
    symbol: String,
    #[serde(default)]
    contracts: Vec<serde_json::Value>,
    #[serde(rename = "costUsd", default)]
    cost_usd: f64,
}

#[derive(Debug, Deserialize)]
struct CryptoSeriesResponse {
    series: CryptoSeries,
    #[serde(rename = "costUsd", default)]
    cost_usd: f64,
}

#[derive(Debug, Deserialize)]
struct CryptoSeries {
    symbol: String,
    market: String,
    series: Vec<CryptoSeriesPoint>,
}

#[derive(Debug, Deserialize)]
struct CryptoSeriesPoint {
    date: String,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: f64,
}

#[derive(Debug, Deserialize)]
struct CommodityResponse {
    series: CommoditySeries,
    #[serde(rename = "costUsd", default)]
    cost_usd: f64,
}

#[derive(Debug, Deserialize)]
struct CommoditySeries {
    commodity: String,
    interval: String,
    #[serde(default)]
    unit: String,
    series: Vec<CommodityPoint>,
}

#[derive(Debug, Deserialize)]
struct CommodityPoint {
    date: String,
    #[serde(default)]
    value: Option<f64>,
}

// ── StockQuoteTool ──────────────────────────────────────────────────

/// Latest quote for a stock or index (e.g. `AAPL`, `SPY`).
pub struct StockQuoteTool {
    client: Arc<IntegrationClient>,
}

impl StockQuoteTool {
    pub fn new(client: Arc<IntegrationClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for StockQuoteTool {
    fn name(&self) -> &str {
        "stock_quote"
    }

    fn description(&self) -> &str {
        "Latest price for a stock or index ticker (e.g. AAPL, MSFT, SPY). \
         Returns price, open/high/low, volume, previous close, and percent change. \
         For crypto prices (e.g. BTC, ETH) or FX rates, use `stock_exchange_rate` instead."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "symbol": {
                    "type": "string",
                    "description": "Stock or index ticker, e.g. AAPL, MSFT, SPY"
                }
            },
            "required": ["symbol"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let symbol = args
            .get("symbol")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: symbol"))?;

        tracing::info!("[stock_quote] symbol={}", symbol);

        let body = json!({ "symbol": symbol });
        match self.client.post::<QuoteResponse>(PATH_QUOTE, &body).await {
            Ok(resp) => {
                let q = &resp.quote;
                let mut out = format!(
                    "{} — ${:.4}\n  open ${:.4}  high ${:.4}  low ${:.4}\n  prev close ${:.4}  change {:+.4} ({})\n  volume {}",
                    q.symbol,
                    q.price,
                    q.open,
                    q.high,
                    q.low,
                    q.previous_close,
                    q.change,
                    q.change_percent,
                    q.volume,
                );
                if !q.latest_trading_day.is_empty() {
                    out.push_str(&format!("\n  latest trading day {}", q.latest_trading_day));
                }
                out.push_str(&format!("\n\nCost: ${:.4}", resp.cost_usd));
                Ok(ToolResult::success(out))
            }
            Err(e) => Ok(ToolResult::error(format!("Stock quote failed: {e}"))),
        }
    }
}

// ── StockExchangeRateTool ───────────────────────────────────────────

/// Realtime exchange rate for FX or crypto (e.g. BTC/USD, EUR/USD).
pub struct StockExchangeRateTool {
    client: Arc<IntegrationClient>,
}

impl StockExchangeRateTool {
    pub fn new(client: Arc<IntegrationClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for StockExchangeRateTool {
    fn name(&self) -> &str {
        "stock_exchange_rate"
    }

    fn description(&self) -> &str {
        "Realtime exchange rate between two currencies — works for both FX (USD, EUR, JPY, …) \
         and digital currencies (BTC, ETH, …). Use this for questions like \
         \"what is the price of BTC?\" (BTC → USD) or \"how much is 1 EUR in USD?\". \
         Returns rate plus bid/ask when available."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "from_currency": {
                    "type": "string",
                    "description": "Source currency (fiat or crypto), e.g. BTC, ETH, USD, EUR"
                },
                "to_currency": {
                    "type": "string",
                    "description": "Target currency, e.g. USD"
                }
            },
            "required": ["from_currency", "to_currency"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let from = args
            .get("from_currency")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: from_currency"))?;
        let to = args
            .get("to_currency")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: to_currency"))?;

        tracing::info!("[stock_exchange_rate] {}->{}", from, to);

        let body = json!({ "fromCurrency": from, "toCurrency": to });
        match self
            .client
            .post::<ExchangeRateResponse>(PATH_EXCHANGE_RATE, &body)
            .await
        {
            Ok(resp) => {
                let r = &resp.rate;
                let mut out = format!("{}/{} = {}\n", r.from_currency, r.to_currency, r.rate);
                if let Some(bid) = r.bid {
                    out.push_str(&format!("  bid {}\n", bid));
                }
                if let Some(ask) = r.ask {
                    out.push_str(&format!("  ask {}\n", ask));
                }
                if !r.last_refreshed.is_empty() {
                    out.push_str(&format!(
                        "  last refreshed {} {}\n",
                        r.last_refreshed, r.time_zone
                    ));
                }
                out.push_str(&format!("\nCost: ${:.4}", resp.cost_usd));
                Ok(ToolResult::success(out))
            }
            Err(e) => Ok(ToolResult::error(format!("Exchange rate failed: {e}"))),
        }
    }
}

// ── StockOptionsTool ────────────────────────────────────────────────

/// Realtime options chain for a symbol.
pub struct StockOptionsTool {
    client: Arc<IntegrationClient>,
}

impl StockOptionsTool {
    pub fn new(client: Arc<IntegrationClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for StockOptionsTool {
    fn name(&self) -> &str {
        "stock_options"
    }

    fn description(&self) -> &str {
        "Realtime options chain for a stock symbol. Returns active option contracts \
         (calls and puts) with strike, expiration, last/bid/ask, volume, and open interest. \
         Set `require_greeks = true` to also include delta, gamma, theta, vega, rho, and IV."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "symbol": { "type": "string", "description": "Stock ticker, e.g. AAPL" },
                "require_greeks": {
                    "type": "boolean",
                    "description": "Include greeks and implied volatility (default false)",
                    "default": false
                }
            },
            "required": ["symbol"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let symbol = args
            .get("symbol")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: symbol"))?;
        let require_greeks = args
            .get("require_greeks")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        tracing::info!(
            "[stock_options] symbol={} greeks={}",
            symbol,
            require_greeks
        );

        let body = json!({ "symbol": symbol, "requireGreeks": require_greeks });
        match self
            .client
            .post::<OptionsResponse>(PATH_OPTIONS, &body)
            .await
        {
            Ok(resp) => {
                let total = resp.contracts.len();
                let mut lines = vec![format!(
                    "{} options chain — {} active contracts",
                    resp.symbol, total
                )];
                for c in resp.contracts.iter().take(20) {
                    let typ = c.get("type").and_then(|v| v.as_str()).unwrap_or("?");
                    let exp = c.get("expiration").and_then(|v| v.as_str()).unwrap_or("?");
                    let strike = c.get("strike").and_then(|v| v.as_str()).unwrap_or("?");
                    let last = c.get("last").and_then(|v| v.as_str()).unwrap_or("");
                    let bid = c.get("bid").and_then(|v| v.as_str()).unwrap_or("");
                    let ask = c.get("ask").and_then(|v| v.as_str()).unwrap_or("");
                    lines.push(format!(
                        "  {} {} @ {}  last {}  bid {}  ask {}",
                        typ, exp, strike, last, bid, ask
                    ));
                }
                if total > 20 {
                    lines.push(format!("  …and {} more contracts", total - 20));
                }
                lines.push(format!("\nCost: ${:.4}", resp.cost_usd));
                Ok(ToolResult::success(lines.join("\n")))
            }
            Err(e) => Ok(ToolResult::error(format!("Stock options failed: {e}"))),
        }
    }
}

// ── StockCryptoSeriesTool ───────────────────────────────────────────

/// Daily OHLCV series for a crypto pair (e.g. BTC/USD historical).
pub struct StockCryptoSeriesTool {
    client: Arc<IntegrationClient>,
}

impl StockCryptoSeriesTool {
    pub fn new(client: Arc<IntegrationClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for StockCryptoSeriesTool {
    fn name(&self) -> &str {
        "stock_crypto_series"
    }

    fn description(&self) -> &str {
        "Daily OHLCV series for a digital currency (e.g. BTC, ETH). \
         Returns the most recent N days of open / high / low / close / volume. \
         For a single spot price, prefer `stock_exchange_rate`."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "symbol": { "type": "string", "description": "Crypto symbol, e.g. BTC, ETH" },
                "market": {
                    "type": "string",
                    "description": "Quote market, default USD",
                    "default": "USD"
                },
                "limit": {
                    "type": "integer",
                    "description": "Max points to return (most recent first), default 30",
                    "minimum": 1,
                    "maximum": 1000,
                    "default": 30
                }
            },
            "required": ["symbol"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let symbol = args
            .get("symbol")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: symbol"))?;

        let mut body = json!({ "symbol": symbol });
        if let Some(m) = args.get("market").and_then(|v| v.as_str()) {
            body["market"] = json!(m);
        }
        if let Some(l) = args.get("limit").and_then(|v| v.as_u64()) {
            body["limit"] = json!(l.clamp(1, 1000));
        }

        tracing::info!("[stock_crypto_series] symbol={}", symbol);

        match self
            .client
            .post::<CryptoSeriesResponse>(PATH_CRYPTO_SERIES, &body)
            .await
        {
            Ok(resp) => {
                let s = &resp.series;
                let mut lines = vec![format!(
                    "{}/{} — {} day(s)",
                    s.symbol,
                    s.market,
                    s.series.len()
                )];
                for p in s.series.iter().take(30) {
                    lines.push(format!(
                        "  {}  o {:.2}  h {:.2}  l {:.2}  c {:.2}  v {:.2}",
                        p.date, p.open, p.high, p.low, p.close, p.volume
                    ));
                }
                if s.series.len() > 30 {
                    lines.push(format!("  …and {} more rows", s.series.len() - 30));
                }
                lines.push(format!("\nCost: ${:.4}", resp.cost_usd));
                Ok(ToolResult::success(lines.join("\n")))
            }
            Err(e) => Ok(ToolResult::error(format!("Crypto series failed: {e}"))),
        }
    }
}

// ── StockCommodityTool ──────────────────────────────────────────────

/// Commodity / futures price series — WTI, BRENT, NATURAL_GAS.
pub struct StockCommodityTool {
    client: Arc<IntegrationClient>,
}

impl StockCommodityTool {
    pub fn new(client: Arc<IntegrationClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for StockCommodityTool {
    fn name(&self) -> &str {
        "stock_commodity"
    }

    fn description(&self) -> &str {
        "Commodity / futures price series — WTI crude, Brent crude, or natural gas. \
         Returns dated value points at daily, weekly, or monthly granularity."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "commodity": {
                    "type": "string",
                    "enum": ["WTI", "BRENT", "NATURAL_GAS"],
                    "description": "Commodity to fetch"
                },
                "interval": {
                    "type": "string",
                    "enum": ["daily", "weekly", "monthly"],
                    "description": "Sampling interval (default daily)",
                    "default": "daily"
                },
                "limit": {
                    "type": "integer",
                    "description": "Max points (default 30)",
                    "minimum": 1,
                    "maximum": 1000,
                    "default": 30
                }
            },
            "required": ["commodity"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let commodity = args
            .get("commodity")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: commodity"))?;

        let mut body = json!({ "commodity": commodity });
        if let Some(i) = args.get("interval").and_then(|v| v.as_str()) {
            body["interval"] = json!(i);
        }
        if let Some(l) = args.get("limit").and_then(|v| v.as_u64()) {
            body["limit"] = json!(l.clamp(1, 1000));
        }

        tracing::info!("[stock_commodity] {}", commodity);

        match self
            .client
            .post::<CommodityResponse>(PATH_COMMODITY, &body)
            .await
        {
            Ok(resp) => {
                let s = &resp.series;
                let mut lines = vec![format!(
                    "{} ({}) — {} ({})",
                    s.commodity,
                    s.interval,
                    if s.unit.is_empty() { "value" } else { &s.unit },
                    s.series.len()
                )];
                for p in s.series.iter().take(30) {
                    let v = p
                        .value
                        .map(|v| format!("{:.4}", v))
                        .unwrap_or_else(|| "n/a".into());
                    lines.push(format!("  {}  {}", p.date, v));
                }
                if s.series.len() > 30 {
                    lines.push(format!("  …and {} more rows", s.series.len() - 30));
                }
                lines.push(format!("\nCost: ${:.4}", resp.cost_usd));
                Ok(ToolResult::success(lines.join("\n")))
            }
            Err(e) => Ok(ToolResult::error(format!("Commodity series failed: {e}"))),
        }
    }
}

#[cfg(test)]
#[path = "stock_prices_tests.rs"]
mod tests;
