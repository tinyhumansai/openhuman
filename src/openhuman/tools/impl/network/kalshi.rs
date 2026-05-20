use super::kalshi_auth::sign_kalshi_headers;
use crate::openhuman::config::{KalshiConfig, KalshiCredentials};
use crate::openhuman::security::policy::ToolOperation;
use crate::openhuman::security::SecurityPolicy;
use crate::openhuman::tools::traits::{Tool, ToolCategory, ToolResult};
use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use uuid::Uuid;

const MAX_RETRY_ATTEMPTS: usize = 3;
const RETRY_BACKOFF_MS: u64 = 500;
const CONNECT_TIMEOUT_SECS: u64 = 10;
const MAX_ERROR_BODY_CHARS: usize = 240;

pub struct KalshiTool {
    base_url: String,
    signing_path_prefix: String,
    http: Client,
    security: Arc<SecurityPolicy>,
    timeout: Duration,
    credentials: Option<KalshiCredentials>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum KalshiRequest {
    ListMarkets {
        #[serde(default)]
        status: Option<String>,
        #[serde(default)]
        limit: Option<u64>,
        #[serde(default)]
        cursor: Option<String>,
        #[serde(default)]
        event_ticker: Option<String>,
        #[serde(default)]
        series_ticker: Option<String>,
        #[serde(default)]
        min_close_ts: Option<u64>,
        #[serde(default)]
        max_close_ts: Option<u64>,
    },
    GetMarket {
        ticker: String,
    },
    ListEvents {
        #[serde(default)]
        status: Option<String>,
        #[serde(default)]
        limit: Option<u64>,
        #[serde(default)]
        cursor: Option<String>,
    },
    GetEvent {
        event_ticker: String,
    },
    GetOrderbook {
        ticker: String,
        #[serde(default)]
        depth: Option<u64>,
    },
    GetPositions,
    GetBalance,
    GetOpenOrders {
        #[serde(default)]
        limit: Option<u64>,
        #[serde(default)]
        cursor: Option<String>,
    },
    GetFills {
        #[serde(default)]
        limit: Option<u64>,
        #[serde(default)]
        cursor: Option<String>,
    },
    PlaceOrder {
        ticker: String,
        side: String,
        order_action: String,
        count: u64,
        #[serde(rename = "type")]
        order_type: String,
        #[serde(default)]
        yes_price: Option<u64>,
        #[serde(default)]
        no_price: Option<u64>,
        #[serde(default)]
        expiration_ts: Option<u64>,
        #[serde(default)]
        approved: Option<bool>,
    },
    CancelOrder {
        order_id: String,
        #[serde(default)]
        approved: Option<bool>,
    },
}

impl KalshiTool {
    pub fn new(config: &KalshiConfig, security: Arc<SecurityPolicy>) -> Self {
        let timeout = Duration::from_secs(config.timeout_secs.max(1));

        let builder = reqwest::Client::builder()
            .timeout(timeout)
            .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
            .redirect(reqwest::redirect::Policy::none());
        let builder =
            crate::openhuman::config::apply_runtime_proxy_to_builder(builder, "tool.kalshi");

        let http = builder.build().unwrap_or_else(|err| {
            panic!(
                "[kalshi] failed to build HTTP client (proxy/timeout configuration): {err}. \
                 Refusing to fall back to Client::new() — silent fallback hides the misconfiguration \
                 and produces requests that bypass the configured proxy + timeouts."
            )
        });

        Self {
            base_url: normalize_base_url(
                &config.base_url,
                "https://api.elections.kalshi.com/trade-api/v2",
            ),
            signing_path_prefix: signing_path_prefix(&config.base_url),
            http,
            security,
            timeout,
            credentials: config.credentials.clone(),
        }
    }

    async fn handle_request(&self, request: KalshiRequest) -> Result<Value> {
        match request {
            KalshiRequest::ListMarkets {
                status,
                limit,
                cursor,
                event_ticker,
                series_ticker,
                min_close_ts,
                max_close_ts,
            } => {
                let mut query = Vec::new();
                push_optional_string(
                    &mut query,
                    "status",
                    non_empty(status.as_deref()).or(Some("open")),
                );
                push_optional_u64(&mut query, "limit", limit);
                push_optional_string(&mut query, "cursor", non_empty(cursor.as_deref()));
                push_optional_string(
                    &mut query,
                    "event_ticker",
                    non_empty(event_ticker.as_deref()),
                );
                push_optional_string(
                    &mut query,
                    "series_ticker",
                    non_empty(series_ticker.as_deref()),
                );
                push_optional_u64(&mut query, "min_close_ts", min_close_ts);
                push_optional_u64(&mut query, "max_close_ts", max_close_ts);

                let data = self.get_json("/markets", &query, None).await?;
                Ok(json!({
                    "action": "list_markets",
                    "source": "kalshi",
                    "data": data,
                }))
            }
            KalshiRequest::GetMarket { ticker } => {
                let ticker = non_empty(Some(ticker.as_str()))
                    .ok_or_else(|| anyhow::anyhow!("'ticker' cannot be empty"))?;
                let data = self
                    .get_json(&format!("/markets/{ticker}"), &[], None)
                    .await?;
                Ok(json!({
                    "action": "get_market",
                    "source": "kalshi",
                    "ticker": ticker,
                    "data": data,
                }))
            }
            KalshiRequest::ListEvents {
                status,
                limit,
                cursor,
            } => {
                let mut query = Vec::new();
                push_optional_string(
                    &mut query,
                    "status",
                    non_empty(status.as_deref()).or(Some("open")),
                );
                push_optional_u64(&mut query, "limit", limit);
                push_optional_string(&mut query, "cursor", non_empty(cursor.as_deref()));
                let data = self.get_json("/events", &query, None).await?;
                Ok(json!({
                    "action": "list_events",
                    "source": "kalshi",
                    "data": data,
                }))
            }
            KalshiRequest::GetEvent { event_ticker } => {
                let event_ticker = non_empty(Some(event_ticker.as_str()))
                    .ok_or_else(|| anyhow::anyhow!("'event_ticker' cannot be empty"))?;
                let data = self
                    .get_json(&format!("/events/{event_ticker}"), &[], None)
                    .await?;
                Ok(json!({
                    "action": "get_event",
                    "source": "kalshi",
                    "event_ticker": event_ticker,
                    "data": data,
                }))
            }
            KalshiRequest::GetOrderbook { ticker, depth } => {
                let ticker = non_empty(Some(ticker.as_str()))
                    .ok_or_else(|| anyhow::anyhow!("'ticker' cannot be empty"))?;
                let mut query = Vec::new();
                push_optional_u64(&mut query, "depth", depth);
                let data = self
                    .get_json(&format!("/markets/{ticker}/orderbook"), &query, None)
                    .await?;
                Ok(json!({
                    "action": "get_orderbook",
                    "source": "kalshi",
                    "ticker": ticker,
                    "data": data,
                }))
            }
            KalshiRequest::GetPositions => {
                let data = self.get_signed_json("/portfolio/positions", &[]).await?;
                Ok(json!({
                    "action": "get_positions",
                    "source": "kalshi",
                    "data": data,
                }))
            }
            KalshiRequest::GetBalance => {
                let data = self.get_signed_json("/portfolio/balance", &[]).await?;
                Ok(json!({
                    "action": "get_balance",
                    "source": "kalshi",
                    "data": data,
                }))
            }
            KalshiRequest::GetOpenOrders { limit, cursor } => {
                let mut query = vec![("status".to_string(), "resting".to_string())];
                push_optional_u64(&mut query, "limit", limit);
                push_optional_string(&mut query, "cursor", non_empty(cursor.as_deref()));
                let data = self.get_signed_json("/portfolio/orders", &query).await?;
                Ok(json!({
                    "action": "get_open_orders",
                    "source": "kalshi",
                    "data": data,
                }))
            }
            KalshiRequest::GetFills { limit, cursor } => {
                let mut query = Vec::new();
                push_optional_u64(&mut query, "limit", limit);
                push_optional_string(&mut query, "cursor", non_empty(cursor.as_deref()));
                let data = self.get_signed_json("/portfolio/fills", &query).await?;
                Ok(json!({
                    "action": "get_fills",
                    "source": "kalshi",
                    "data": data,
                }))
            }
            KalshiRequest::PlaceOrder {
                ticker,
                side,
                order_action,
                count,
                order_type,
                yes_price,
                no_price,
                expiration_ts,
                approved,
            } => {
                self.security
                    .enforce_tool_operation(ToolOperation::Act, "kalshi.place_order")
                    .map_err(anyhow::Error::msg)?;
                require_write_approval(approved)?;

                let ticker = non_empty(Some(ticker.as_str()))
                    .ok_or_else(|| anyhow::anyhow!("'ticker' cannot be empty"))?;
                let side = normalize_yes_no(&side)?;
                let order_action = normalize_buy_sell(&order_action)?;
                let order_type = normalize_order_type(&order_type)?;
                if count == 0 {
                    anyhow::bail!("'count' must be greater than zero");
                }

                let yes_price = validate_price_cents("yes_price", yes_price)?;
                let no_price = validate_price_cents("no_price", no_price)?;
                if order_type == "limit" && yes_price.is_none() && no_price.is_none() {
                    anyhow::bail!("limit orders require at least one of 'yes_price' or 'no_price'");
                }

                let client_order_id = Uuid::new_v4().to_string();
                let mut body = json!({
                    "ticker": ticker,
                    "client_order_id": client_order_id,
                    "side": side,
                    "action": order_action,
                    "count": count,
                    "type": order_type,
                });
                if let Value::Object(ref mut map) = body {
                    if let Some(value) = yes_price {
                        map.insert("yes_price".to_string(), Value::from(value));
                    }
                    if let Some(value) = no_price {
                        map.insert("no_price".to_string(), Value::from(value));
                    }
                    if let Some(value) = expiration_ts {
                        map.insert("expiration_ts".to_string(), Value::from(value));
                    }
                }

                let data = self.post_signed_json("/portfolio/orders", body).await?;
                Ok(json!({
                    "action": "place_order",
                    "source": "kalshi",
                    "client_order_id": client_order_id,
                    "data": data,
                }))
            }
            KalshiRequest::CancelOrder { order_id, approved } => {
                self.security
                    .enforce_tool_operation(ToolOperation::Act, "kalshi.cancel_order")
                    .map_err(anyhow::Error::msg)?;
                require_write_approval(approved)?;

                let order_id = non_empty(Some(order_id.as_str()))
                    .ok_or_else(|| anyhow::anyhow!("'order_id' cannot be empty"))?;
                let path = format!("/portfolio/orders/{order_id}");
                let data = self.delete_signed_json(&path).await?;
                Ok(json!({
                    "action": "cancel_order",
                    "source": "kalshi",
                    "order_id": order_id,
                    "data": data,
                }))
            }
        }
    }

    async fn get_signed_json(&self, path: &str, query: &[(String, String)]) -> Result<Value> {
        let credentials = self.require_credentials()?;
        ensure_https(&self.base_url)?;
        let signed_path = format!("{}{}", self.signing_path_prefix, path);
        let headers = sign_kalshi_headers(credentials, "GET", &signed_path, None)?;
        self.get_json(path, query, Some(headers)).await
    }

    fn require_credentials(&self) -> Result<&KalshiCredentials> {
        self.credentials
            .as_ref()
            .filter(|creds| creds.is_complete())
            .ok_or_else(|| anyhow::anyhow!("Kalshi credentials are required for this action"))
    }

    async fn post_signed_json(&self, path: &str, body: Value) -> Result<Value> {
        let credentials = self.require_credentials()?;
        ensure_https(&self.base_url)?;
        let body_raw = serde_json::to_string(&body).context("Failed to serialize Kalshi body")?;
        let signed_path = format!("{}{}", self.signing_path_prefix, path);
        let mut headers = sign_kalshi_headers(credentials, "POST", &signed_path, Some(&body_raw))?;
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        self.post_json_raw(path, Some(headers), body_raw).await
    }

    async fn delete_signed_json(&self, path: &str) -> Result<Value> {
        let credentials = self.require_credentials()?;
        ensure_https(&self.base_url)?;
        let signed_path = format!("{}{}", self.signing_path_prefix, path);
        let headers = sign_kalshi_headers(credentials, "DELETE", &signed_path, None)?;
        self.delete_json(path, Some(headers)).await
    }

    async fn get_json(
        &self,
        path: &str,
        query: &[(String, String)],
        headers: Option<HeaderMap>,
    ) -> Result<Value> {
        self.send_with_retry(reqwest::Method::GET, path, query, headers, None)
            .await
    }

    async fn post_json_raw(
        &self,
        path: &str,
        headers: Option<HeaderMap>,
        body_raw: String,
    ) -> Result<Value> {
        self.send_with_retry(reqwest::Method::POST, path, &[], headers, Some(body_raw))
            .await
    }

    async fn delete_json(&self, path: &str, headers: Option<HeaderMap>) -> Result<Value> {
        self.send_with_retry(reqwest::Method::DELETE, path, &[], headers, None)
            .await
    }

    async fn send_with_retry(
        &self,
        method: reqwest::Method,
        path: &str,
        query: &[(String, String)],
        headers: Option<HeaderMap>,
        body: Option<String>,
    ) -> Result<Value> {
        let url = format!("{}{}", self.base_url, path);
        let method_label = method.as_str().to_string();

        for attempt in 1..=MAX_RETRY_ATTEMPTS {
            let mut request = self.http.request(method.clone(), &url);
            if !query.is_empty() {
                request = request.query(query);
            }
            if let Some(h) = headers.as_ref() {
                request = request.headers(h.clone());
            }
            if let Some(b) = body.as_ref() {
                request = request.body(b.clone());
            }

            let response = match request.send().await {
                Ok(resp) => resp,
                Err(err) => {
                    if err.is_timeout() {
                        anyhow::bail!(
                            "Kalshi request timed out after {}s: {method_label} {path}",
                            self.timeout.as_secs()
                        );
                    }

                    if attempt < MAX_RETRY_ATTEMPTS {
                        sleep(Duration::from_millis(RETRY_BACKOFF_MS)).await;
                        continue;
                    }

                    anyhow::bail!(
                        "Kalshi transient transport error for {method_label} {path}: {err} (url: {url})"
                    );
                }
            };

            let status = response.status();
            if status.is_success() {
                let text = response
                    .text()
                    .await
                    .unwrap_or_else(|_| String::from("null"));
                if text.trim().is_empty() {
                    return Ok(Value::Null);
                }
                return serde_json::from_str(&text)
                    .with_context(|| format!("Failed to deserialize Kalshi response: {path}"));
            }

            let body = response
                .text()
                .await
                .unwrap_or_else(|_| String::from("<failed to read response body>"));
            let detail = summarize_error_body(&body);

            if status.is_client_error() && status != StatusCode::TOO_MANY_REQUESTS {
                anyhow::bail!(
                    "Kalshi client error {} for {method_label} {path}: {detail}",
                    status.as_u16()
                );
            }

            let transient = status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error();
            if transient && attempt < MAX_RETRY_ATTEMPTS {
                sleep(Duration::from_millis(RETRY_BACKOFF_MS)).await;
                continue;
            }

            if status == StatusCode::TOO_MANY_REQUESTS {
                anyhow::bail!(
                    "Kalshi transient rate-limit error {} for {method_label} {path} after {attempt} attempts: {detail}",
                    status.as_u16()
                );
            }

            if status.is_server_error() {
                anyhow::bail!(
                    "Kalshi transient server error {} for {method_label} {path} after {attempt} attempts: {detail}",
                    status.as_u16()
                );
            }

            anyhow::bail!(
                "Kalshi HTTP error {} for {method_label} {path}: {detail}",
                status.as_u16()
            );
        }

        anyhow::bail!("Kalshi request failed: retry budget exhausted")
    }
}

#[async_trait]
impl Tool for KalshiTool {
    fn name(&self) -> &str {
        "kalshi"
    }

    fn description(&self) -> &str {
        "Browse Kalshi market data via the trade-api v2 public endpoints."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "description": "Kalshi action to run.",
                    "enum": [
                        "list_markets",
                        "get_market",
                        "list_events",
                        "get_event",
                        "get_orderbook",
                        "get_positions",
                        "get_balance",
                        "get_open_orders",
                        "get_fills",
                        "place_order",
                        "cancel_order"
                    ]
                },
                "ticker": {
                    "type": "string",
                    "description": "Kalshi market ticker for get_market/get_orderbook."
                },
                "event_ticker": {
                    "type": "string",
                    "description": "Kalshi event ticker for get_event or list_markets filter."
                },
                "series_ticker": {
                    "type": "string",
                    "description": "Optional series filter for list_markets."
                },
                "status": {
                    "type": "string",
                    "description": "Optional status filter (default: open)."
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Pagination limit for list actions."
                },
                "cursor": {
                    "type": "string",
                    "description": "Cursor token for paginated list responses."
                },
                "min_close_ts": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "Optional minimum close timestamp (seconds)."
                },
                "max_close_ts": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "Optional maximum close timestamp (seconds)."
                },
                "depth": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Orderbook depth for get_orderbook."
                },
                "side": {
                    "type": "string",
                    "enum": ["yes", "no", "YES", "NO"],
                    "description": "Order side for place_order."
                },
                "order_action": {
                    "type": "string",
                    "enum": ["buy", "sell", "BUY", "SELL"],
                    "description": "Order action for place_order."
                },
                "type": {
                    "type": "string",
                    "enum": ["limit", "market", "LIMIT", "MARKET"],
                    "description": "Order type for place_order."
                },
                "count": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Contract count for place_order."
                },
                "yes_price": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 99,
                    "description": "YES price in cents for place_order."
                },
                "no_price": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 99,
                    "description": "NO price in cents for place_order."
                },
                "expiration_ts": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "Optional expiration timestamp in UNIX seconds."
                },
                "order_id": {
                    "type": "string",
                    "description": "Order id for cancel_order."
                },
                "approved": {
                    "type": "boolean",
                    "description": "Required=true for write actions (place_order, cancel_order)."
                }
            },
            "required": ["action"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Skill
    }

    fn is_concurrency_safe(&self, _args: &Value) -> bool {
        !matches!(
            _args.get("action").and_then(Value::as_str),
            Some("place_order") | Some("cancel_order")
        )
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        if self.security.is_rate_limited() {
            return Ok(ToolResult::error(
                "Rate limit exceeded: too many actions in the last hour",
            ));
        }

        if !self.security.record_action() {
            return Ok(ToolResult::error(
                "Rate limit exceeded: action budget exhausted",
            ));
        }

        let request: KalshiRequest = serde_json::from_value(args)
            .context("Invalid kalshi request: unable to parse parameters")?;

        match self.handle_request(request).await {
            Ok(payload) => Ok(ToolResult::json(payload)),
            Err(err) => Ok(ToolResult::error(err.to_string())),
        }
    }
}

fn normalize_base_url(raw: &str, fallback: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return fallback.to_string();
    }
    trimmed.trim_end_matches('/').to_string()
}

fn signing_path_prefix(raw_base_url: &str) -> String {
    let normalized = normalize_base_url(
        raw_base_url,
        "https://api.elections.kalshi.com/trade-api/v2",
    );
    let Ok(parsed) = url::Url::parse(&normalized) else {
        return String::new();
    };
    let path = parsed.path().trim_end_matches('/');
    match path {
        "" | "/" => String::new(),
        other => other.to_string(),
    }
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|v| !v.is_empty())
}

fn push_optional_string(query: &mut Vec<(String, String)>, key: &str, value: Option<&str>) {
    if let Some(value) = value {
        query.push((key.to_string(), value.to_string()));
    }
}

fn push_optional_u64(query: &mut Vec<(String, String)>, key: &str, value: Option<u64>) {
    if let Some(value) = value {
        query.push((key.to_string(), value.to_string()));
    }
}

fn summarize_error_body(body: &str) -> String {
    let compact = body.trim().replace('\n', " ");
    if compact.is_empty() {
        "empty response body".to_string()
    } else {
        crate::openhuman::util::truncate_with_ellipsis(&compact, MAX_ERROR_BODY_CHARS)
    }
}

fn normalize_yes_no(value: &str) -> Result<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "yes" => Ok("yes"),
        "no" => Ok("no"),
        _ => anyhow::bail!("Invalid 'side'. Expected one of: yes, no"),
    }
}

fn normalize_buy_sell(value: &str) -> Result<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "buy" => Ok("buy"),
        "sell" => Ok("sell"),
        _ => anyhow::bail!("Invalid 'action'. Expected one of: buy, sell"),
    }
}

fn normalize_order_type(value: &str) -> Result<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "limit" => Ok("limit"),
        "market" => Ok("market"),
        _ => anyhow::bail!("Invalid 'type'. Expected one of: limit, market"),
    }
}

fn validate_price_cents(field: &str, value: Option<u64>) -> Result<Option<u64>> {
    let Some(value) = value else {
        return Ok(None);
    };
    if !(1..=99).contains(&value) {
        anyhow::bail!("'{field}' must be in the range 1..=99 cents");
    }
    Ok(Some(value))
}

fn ensure_https(url: &str) -> Result<()> {
    if url.starts_with("https://") {
        return Ok(());
    }
    if url.starts_with("http://127.0.0.1")
        || url.starts_with("http://[::1]")
        || url.starts_with("http://localhost")
    {
        return Ok(());
    }
    anyhow::bail!(
        "Refusing to transmit Kalshi credentials over non-HTTPS URL: \
         URL scheme must be https (loopback http allowed for local mock)"
    )
}

fn require_write_approval(approved: Option<bool>) -> Result<()> {
    if approved.unwrap_or(false) {
        return Ok(());
    }

    anyhow::bail!(
        "Kalshi write requires explicit user approval. Re-invoke with arguments.approved = true after confirming with the user."
    )
}

#[cfg(test)]
#[path = "kalshi_tests.rs"]
mod tests;
