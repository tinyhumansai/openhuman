use crate::openhuman::config::{KalshiConfig, KalshiCredentials};
use crate::openhuman::security::SecurityPolicy;
use crate::openhuman::tools::traits::{Tool, ToolCategory, ToolResult};
use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::header::HeaderMap;
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;

const MAX_RETRY_ATTEMPTS: usize = 3;
const RETRY_BACKOFF_MS: u64 = 500;
const CONNECT_TIMEOUT_SECS: u64 = 10;
const MAX_ERROR_BODY_CHARS: usize = 240;

pub struct KalshiTool {
    base_url: String,
    http: Client,
    security: Arc<SecurityPolicy>,
    timeout: Duration,
    #[allow(dead_code)]
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
            base_url: normalize_base_url(&config.base_url, "https://api.elections.kalshi.com/trade-api/v2"),
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
        }
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
                        "get_orderbook"
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
                }
            },
            "required": ["action"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Skill
    }

    fn is_concurrency_safe(&self, _args: &Value) -> bool {
        true
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
