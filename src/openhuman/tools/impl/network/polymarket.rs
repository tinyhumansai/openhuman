use crate::openhuman::config::PolymarketConfig;
use crate::openhuman::security::SecurityPolicy;
use crate::openhuman::tools::traits::{Tool, ToolCategory, ToolResult};
use anyhow::Context;
use async_trait::async_trait;
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

/// Read-only Polymarket market-browse tool (Gamma + CLOB public APIs).
pub struct PolymarketTool {
    gamma_base_url: String,
    clob_base_url: String,
    http: Client,
    security: Arc<SecurityPolicy>,
    timeout: Duration,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum PolymarketRequest {
    ListMarkets {
        #[serde(default)]
        slug: Option<String>,
        #[serde(default)]
        event_id: Option<String>,
        #[serde(default)]
        limit: Option<u64>,
        #[serde(default)]
        offset: Option<u64>,
        #[serde(default)]
        cursor: Option<String>,
        #[serde(default)]
        active: Option<bool>,
        #[serde(default)]
        closed: Option<bool>,
        #[serde(default)]
        tag: Option<String>,
    },
    GetMarket {
        #[serde(default)]
        market_id: Option<String>,
        #[serde(default)]
        slug: Option<String>,
    },
    ListEvents {
        #[serde(default)]
        event_id: Option<String>,
        #[serde(default)]
        limit: Option<u64>,
        #[serde(default)]
        offset: Option<u64>,
        #[serde(default)]
        cursor: Option<String>,
        #[serde(default)]
        active: Option<bool>,
        #[serde(default)]
        closed: Option<bool>,
        #[serde(default)]
        tag: Option<String>,
    },
    GetOrderbook {
        token_id: String,
    },
    GetPrice {
        token_id: String,
        side: String,
    },
}

impl PolymarketTool {
    pub fn new(config: &PolymarketConfig, security: Arc<SecurityPolicy>) -> Self {
        let timeout = Duration::from_secs(config.timeout_secs.max(1));

        let builder = reqwest::Client::builder()
            .timeout(timeout)
            .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
            .redirect(reqwest::redirect::Policy::none());
        let builder =
            crate::openhuman::config::apply_runtime_proxy_to_builder(builder, "tool.polymarket");

        let http = match builder.build() {
            Ok(client) => client,
            Err(err) => {
                tracing::warn!(
                    reason = %err,
                    "[polymarket] failed to build configured HTTP client, falling back to Client::new()"
                );
                Client::new()
            }
        };

        Self {
            gamma_base_url: normalize_base_url(
                &config.gamma_base_url,
                "https://gamma-api.polymarket.com",
            ),
            clob_base_url: normalize_base_url(&config.clob_base_url, "https://clob.polymarket.com"),
            http,
            security,
            timeout,
        }
    }

    async fn handle_request(&self, request: PolymarketRequest) -> anyhow::Result<Value> {
        match request {
            PolymarketRequest::ListMarkets {
                slug,
                event_id,
                limit,
                offset,
                cursor,
                active,
                closed,
                tag,
            } => {
                let mut query = Vec::new();
                push_optional_string(&mut query, "slug", slug);
                push_optional_string(&mut query, "event_id", event_id);
                push_optional_u64(&mut query, "limit", limit);
                push_optional_u64(&mut query, "offset", offset);
                push_optional_string(&mut query, "cursor", cursor);
                push_optional_bool(&mut query, "active", active);
                push_optional_bool(&mut query, "closed", closed);
                push_optional_string(&mut query, "tag", tag);

                let data = self
                    .get_json(&self.gamma_base_url, "/markets", &query)
                    .await?;
                Ok(json!({
                    "action": "list_markets",
                    "source": "gamma",
                    "data": data,
                }))
            }
            PolymarketRequest::GetMarket { market_id, slug } => {
                if let Some(market_id) = non_empty(market_id.as_deref()) {
                    let path = format!("/markets/{market_id}");
                    let data = self.get_json(&self.gamma_base_url, &path, &[]).await?;
                    Ok(json!({
                        "action": "get_market",
                        "source": "gamma",
                        "lookup": "market_id",
                        "market_id": market_id,
                        "data": data,
                    }))
                } else if let Some(slug) = non_empty(slug.as_deref()) {
                    let data = self
                        .get_json(
                            &self.gamma_base_url,
                            "/markets",
                            &[("slug".to_string(), slug.to_string())],
                        )
                        .await?;

                    let market = first_item_from_collection(data, slug)?;
                    Ok(json!({
                        "action": "get_market",
                        "source": "gamma",
                        "lookup": "slug",
                        "slug": slug,
                        "data": market,
                    }))
                } else {
                    anyhow::bail!("get_market requires either 'market_id' or 'slug'")
                }
            }
            PolymarketRequest::ListEvents {
                event_id,
                limit,
                offset,
                cursor,
                active,
                closed,
                tag,
            } => {
                if let Some(event_id) = non_empty(event_id.as_deref()) {
                    let path = format!("/events/{event_id}");
                    let data = self.get_json(&self.gamma_base_url, &path, &[]).await?;
                    Ok(json!({
                        "action": "list_events",
                        "source": "gamma",
                        "lookup": "event_id",
                        "event_id": event_id,
                        "data": data,
                    }))
                } else {
                    let mut query = Vec::new();
                    push_optional_u64(&mut query, "limit", limit);
                    push_optional_u64(&mut query, "offset", offset);
                    push_optional_string(&mut query, "cursor", cursor);
                    push_optional_bool(&mut query, "active", active);
                    push_optional_bool(&mut query, "closed", closed);
                    push_optional_string(&mut query, "tag", tag);

                    let data = self
                        .get_json(&self.gamma_base_url, "/events", &query)
                        .await?;
                    Ok(json!({
                        "action": "list_events",
                        "source": "gamma",
                        "data": data,
                    }))
                }
            }
            PolymarketRequest::GetOrderbook { token_id } => {
                let token_id = non_empty(Some(token_id.as_str()))
                    .ok_or_else(|| anyhow::anyhow!("'token_id' cannot be empty"))?;

                let data = self
                    .get_json(
                        &self.clob_base_url,
                        "/book",
                        &[("token_id".to_string(), token_id.to_string())],
                    )
                    .await?;

                Ok(json!({
                    "action": "get_orderbook",
                    "source": "clob",
                    "token_id": token_id,
                    "data": data,
                }))
            }
            PolymarketRequest::GetPrice { token_id, side } => {
                let token_id = non_empty(Some(token_id.as_str()))
                    .ok_or_else(|| anyhow::anyhow!("'token_id' cannot be empty"))?;
                let side = normalize_side(&side)?;

                let data = self
                    .get_json(
                        &self.clob_base_url,
                        "/price",
                        &[
                            ("token_id".to_string(), token_id.to_string()),
                            ("side".to_string(), side.to_string()),
                        ],
                    )
                    .await?;

                Ok(json!({
                    "action": "get_price",
                    "source": "clob",
                    "token_id": token_id,
                    "side": side,
                    "data": data,
                }))
            }
        }
    }

    async fn get_json(
        &self,
        base_url: &str,
        path: &str,
        query: &[(String, String)],
    ) -> anyhow::Result<Value> {
        let url = format!("{base_url}{path}");

        for attempt in 1..=MAX_RETRY_ATTEMPTS {
            let mut request = self.http.get(&url);
            if !query.is_empty() {
                request = request.query(query);
            }

            let response = match request.send().await {
                Ok(resp) => resp,
                Err(err) => {
                    if err.is_timeout() {
                        anyhow::bail!(
                            "Polymarket request timed out after {}s: GET {path}",
                            self.timeout.as_secs()
                        );
                    }

                    if attempt < MAX_RETRY_ATTEMPTS {
                        sleep(Duration::from_millis(RETRY_BACKOFF_MS)).await;
                        continue;
                    }

                    anyhow::bail!(
                        "Polymarket transient transport error for GET {path}: {err} (url: {url})"
                    );
                }
            };

            let status = response.status();

            if status.is_success() {
                return response
                    .json::<Value>()
                    .await
                    .with_context(|| format!("Failed to deserialize Polymarket response: {path}"));
            }

            let body = response
                .text()
                .await
                .unwrap_or_else(|_| String::from("<failed to read response body>"));
            let detail = summarize_error_body(&body);

            if status.is_client_error() && status != StatusCode::TOO_MANY_REQUESTS {
                anyhow::bail!(
                    "Polymarket client error {} for GET {path}: {detail}",
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
                    "Polymarket transient rate-limit error {} for GET {path} after {attempt} attempts: {detail}",
                    status.as_u16()
                );
            }

            if status.is_server_error() {
                anyhow::bail!(
                    "Polymarket transient server error {} for GET {path} after {attempt} attempts: {detail}",
                    status.as_u16()
                );
            }

            anyhow::bail!(
                "Polymarket HTTP error {} for GET {path}: {detail}",
                status.as_u16()
            );
        }

        anyhow::bail!("Polymarket request failed: retry budget exhausted")
    }
}

#[async_trait]
impl Tool for PolymarketTool {
    fn name(&self) -> &str {
        "polymarket"
    }

    fn description(&self) -> &str {
        "Browse Polymarket prediction markets (read-only) via Gamma and CLOB public endpoints. Supports market/event listing, market lookup, orderbook, and price checks."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "description": "Polymarket read-only operation to run.",
                    "enum": [
                        "list_markets",
                        "get_market",
                        "list_events",
                        "get_orderbook",
                        "get_price"
                    ]
                },
                "market_id": {
                    "type": "string",
                    "description": "Gamma market id for get_market."
                },
                "event_id": {
                    "type": "string",
                    "description": "Optional event id filter, or exact id for list_events."
                },
                "slug": {
                    "type": "string",
                    "description": "Market slug filter; can also be used to resolve get_market."
                },
                "token_id": {
                    "type": "string",
                    "description": "CLOB token id for get_orderbook/get_price."
                },
                "side": {
                    "type": "string",
                    "description": "Price side for get_price.",
                    "enum": ["buy", "sell"]
                },
                "limit": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "Pagination limit for list_markets/list_events."
                },
                "offset": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "Pagination offset for list_markets/list_events."
                },
                "cursor": {
                    "type": "string",
                    "description": "Cursor token for paginated list responses."
                },
                "active": {
                    "type": "boolean",
                    "description": "Filter active markets/events."
                },
                "closed": {
                    "type": "boolean",
                    "description": "Filter closed markets/events."
                },
                "tag": {
                    "type": "string",
                    "description": "Optional topic/tag filter."
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

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
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

        let request: PolymarketRequest = serde_json::from_value(args)
            .context("Invalid polymarket request: unable to parse parameters")?;

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
    value.map(str::trim).filter(|s| !s.is_empty())
}

fn push_optional_string(query: &mut Vec<(String, String)>, key: &str, value: Option<String>) {
    if let Some(value) = non_empty(value.as_deref()) {
        query.push((key.to_string(), value.to_string()));
    }
}

fn push_optional_u64(query: &mut Vec<(String, String)>, key: &str, value: Option<u64>) {
    if let Some(value) = value {
        query.push((key.to_string(), value.to_string()));
    }
}

fn push_optional_bool(query: &mut Vec<(String, String)>, key: &str, value: Option<bool>) {
    if let Some(value) = value {
        query.push((key.to_string(), value.to_string()));
    }
}

fn normalize_side(side: &str) -> anyhow::Result<&'static str> {
    let side = side.trim().to_ascii_lowercase();
    match side.as_str() {
        "buy" => Ok("buy"),
        "sell" => Ok("sell"),
        _ => anyhow::bail!("Invalid 'side'. Expected one of: buy, sell"),
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

fn first_item_from_collection(data: Value, slug: &str) -> anyhow::Result<Value> {
    match data {
        Value::Array(items) => items
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("No Polymarket market found for slug '{slug}'")),
        Value::Object(mut map) => {
            if let Some(Value::Array(items)) = map.remove("data") {
                return items.into_iter().next().ok_or_else(|| {
                    anyhow::anyhow!("No Polymarket market found for slug '{slug}'")
                });
            }
            Ok(Value::Object(map))
        }
        other => Ok(other),
    }
}

#[cfg(test)]
#[path = "polymarket_tests.rs"]
mod tests;
