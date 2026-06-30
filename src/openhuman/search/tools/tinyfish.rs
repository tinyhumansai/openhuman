//! TinyFish web search, page fetch, and goal-based browser automation tools.
//!
//! **Scope**: All (agent loop + CLI/RPC).
//!
//! **Endpoints**:
//!   - `POST /agent-integrations/tinyfish/search`
//!   - `POST /agent-integrations/tinyfish/fetch`
//!   - `POST /agent-integrations/tinyfish/agent/run`
//!
//! The OpenHuman backend proxies TinyFish calls so API keys, billing, and
//! rate limits stay server-side. Search and Fetch are read-oriented tools;
//! Agent runs execute browser workflows on remote websites.

use crate::openhuman::integrations::IntegrationClient;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolCategory, ToolResult};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

fn truncate_chars(s: &str, max_chars: usize) -> (&str, bool) {
    match s.char_indices().nth(max_chars) {
        Some((byte_idx, _)) => (&s[..byte_idx], true),
        None => (s, false),
    }
}

fn non_empty_string<'a>(args: &'a serde_json::Value, key: &str) -> anyhow::Result<&'a str> {
    args.get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("Missing required parameter: {key}"))
}

fn optional_string<'a>(args: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
}

#[derive(Debug, Deserialize)]
struct TinyFishSearchResponse {
    #[serde(default)]
    results: Vec<TinyFishSearchResult>,
    #[serde(default)]
    total_results: Option<u64>,
    #[serde(rename = "costUsd", default)]
    cost_usd: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct TinyFishSearchResult {
    #[serde(default)]
    position: Option<u64>,
    #[serde(default)]
    site_name: Option<String>,
    #[serde(default)]
    title: String,
    #[serde(default)]
    snippet: String,
    #[serde(default)]
    url: String,
}

#[derive(Debug, Deserialize)]
struct TinyFishFetchResponse {
    #[serde(default)]
    results: Vec<TinyFishFetchResult>,
    #[serde(default)]
    errors: Vec<TinyFishFetchError>,
    #[serde(rename = "costUsd", default)]
    cost_usd: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct TinyFishFetchResult {
    #[serde(default)]
    url: String,
    #[serde(default)]
    final_url: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    text: serde_json::Value,
    #[serde(default)]
    links: Option<Vec<String>>,
    #[serde(default)]
    image_links: Option<Vec<String>>,
    #[serde(default)]
    latency_ms: Option<f64>,
    #[serde(default)]
    format: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TinyFishFetchError {
    #[serde(default)]
    url: String,
    #[serde(default)]
    error: String,
    #[serde(default)]
    status: Option<u16>,
}

#[derive(Debug, Deserialize)]
struct TinyFishAgentRunResponse {
    #[serde(default)]
    run_id: Option<String>,
    #[serde(default)]
    status: String,
    #[serde(default)]
    result: Option<serde_json::Value>,
    #[serde(default)]
    error: Option<serde_json::Value>,
    #[serde(default)]
    num_of_steps: Option<u64>,
    #[serde(rename = "costUsd", default)]
    cost_usd: Option<f64>,
}

fn format_agent_run_response(mut resp: TinyFishAgentRunResponse) -> String {
    tracing::debug!(
        "[tinyfish_agent_run] formatting response status_present={} steps_present={} result_present={} error_present={} cost_present={}",
        !resp.status.is_empty(),
        resp.num_of_steps.is_some(),
        resp.result.is_some(),
        resp.error.is_some(),
        resp.cost_usd.is_some()
    );

    let mut lines = vec!["TinyFish automation finished.".to_string()];
    if !resp.status.is_empty() {
        tracing::debug!("[tinyfish_agent_run] adding status field");
        lines.push(format!("Status: {}", resp.status));
    }
    if let Some(steps) = resp.num_of_steps {
        tracing::debug!(steps = steps, "[tinyfish_agent_run] adding step count");
        lines.push(format!("Steps: {steps}"));
    }
    if let Some(result) = resp.result.take() {
        tracing::debug!("[tinyfish_agent_run] adding result payload");
        lines.push("Result:".to_string());
        lines.push(result.to_string());
    }
    if let Some(error) = resp.error.take() {
        tracing::debug!("[tinyfish_agent_run] adding error payload");
        lines.push("Error:".to_string());
        lines.push(error.to_string());
    }
    if let Some(cost_usd) = resp.cost_usd {
        tracing::debug!(cost_usd = cost_usd, "[tinyfish_agent_run] adding cost");
        lines.push(format!("Cost: ${cost_usd:.4}"));
    }
    lines.join("\n")
}

/// Search the web with TinyFish Search.
pub struct TinyFishSearchTool {
    client: Arc<IntegrationClient>,
}

impl TinyFishSearchTool {
    pub fn new(client: Arc<IntegrationClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for TinyFishSearchTool {
    fn name(&self) -> &str {
        "tinyfish_search"
    }

    fn description(&self) -> &str {
        "Search the web with TinyFish and return ranked results with titles, snippets, \
         URLs, and source sites. Use this when you need search results before fetching \
         or automating a page."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query"
                },
                "location": {
                    "type": "string",
                    "description": "Optional country code for geo-targeted results, e.g. US, GB, FR"
                },
                "language": {
                    "type": "string",
                    "description": "Optional language code, e.g. en, fr"
                },
                "page": {
                    "type": "integer",
                    "description": "Optional result page number, starting at 0",
                    "minimum": 0,
                    "maximum": 10
                },
                "include_thumbnail": {
                    "type": "boolean",
                    "description": "Whether to include thumbnail URLs when available",
                    "default": false
                }
            },
            "required": ["query"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Workflow
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let query = non_empty_string(&args, "query")?;
        let mut body = json!({ "query": query });
        if let Some(location) = optional_string(&args, "location") {
            body["location"] = json!(location);
        }
        if let Some(language) = optional_string(&args, "language") {
            body["language"] = json!(language);
        }
        if let Some(page) = args.get("page").and_then(|v| v.as_u64()) {
            body["page"] = json!(page.min(10));
        }
        if let Some(include_thumbnail) = args.get("include_thumbnail").and_then(|v| v.as_bool()) {
            body["include_thumbnail"] = json!(include_thumbnail);
        }

        tracing::debug!(query = query, "[tinyfish_search] searching");

        match self
            .client
            .post::<TinyFishSearchResponse>("/agent-integrations/tinyfish/search", &body)
            .await
        {
            Ok(resp) => {
                if resp.results.is_empty() {
                    return Ok(ToolResult::success(format!(
                        "No TinyFish search results found for: {query}"
                    )));
                }

                let mut lines = vec![format!(
                    "TinyFish returned {} search result(s) for: {query}",
                    resp.results.len()
                )];
                if let Some(total) = resp.total_results {
                    lines.push(format!("Total results: {total}"));
                }

                for (idx, item) in resp.results.iter().take(10).enumerate() {
                    let position = item.position.unwrap_or((idx + 1) as u64);
                    lines.push(format!("\n{}. {}", position, item.title));
                    if let Some(site_name) = item.site_name.as_deref() {
                        if !site_name.is_empty() {
                            lines.push(format!("   Site: {site_name}"));
                        }
                    }
                    if !item.url.is_empty() {
                        lines.push(format!("   URL: {}", item.url));
                    }
                    if !item.snippet.is_empty() {
                        lines.push(format!("   Snippet: {}", item.snippet));
                    }
                }

                if resp.results.len() > 10 {
                    lines.push("Output truncated to the first 10 results.".to_string());
                }
                if let Some(cost_usd) = resp.cost_usd {
                    lines.push(format!("\nCost: ${cost_usd:.4}"));
                }

                Ok(ToolResult::success(lines.join("\n")))
            }
            Err(e) => {
                tracing::debug!(
                    query = query,
                    error = %e,
                    "[tinyfish_search] request failed"
                );
                Ok(ToolResult::error(format!("TinyFish search failed: {e}")))
            }
        }
    }
}

/// Render web pages with TinyFish Fetch and return extracted content.
pub struct TinyFishFetchTool {
    client: Arc<IntegrationClient>,
}

impl TinyFishFetchTool {
    pub fn new(client: Arc<IntegrationClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for TinyFishFetchTool {
    fn name(&self) -> &str {
        "tinyfish_fetch"
    }

    fn description(&self) -> &str {
        "Render one or more URLs with TinyFish Fetch and extract clean page content. \
         Use this for JavaScript-heavy pages when you already know the URLs."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "urls": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "HTTP or HTTPS URLs to fetch (1-10)",
                    "minItems": 1,
                    "maxItems": 10
                },
                "format": {
                    "type": "string",
                    "enum": ["markdown", "html", "json"],
                    "description": "Output format for extracted content (default markdown)",
                    "default": "markdown"
                },
                "links": {
                    "type": "boolean",
                    "description": "Include absolute outbound links found on each page",
                    "default": false
                },
                "image_links": {
                    "type": "boolean",
                    "description": "Include absolute image URLs found on each page",
                    "default": false
                }
            },
            "required": ["urls"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Workflow
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let urls = args
            .get("urls")
            .and_then(|v| v.as_array())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: urls"))?;
        if urls.is_empty() {
            return Ok(ToolResult::error("urls must contain at least one URL"));
        }

        let mut normalized_urls = Vec::with_capacity(urls.len().min(10));
        for (idx, url) in urls.iter().take(10).enumerate() {
            match url.as_str() {
                Some(s) if !s.trim().is_empty() => normalized_urls.push(s),
                Some(_) => return Ok(ToolResult::error(format!("urls[{idx}] is empty"))),
                None => return Ok(ToolResult::error(format!("urls[{idx}] is not a string"))),
            }
        }

        let mut body = json!({ "urls": normalized_urls });
        if let Some(format) = optional_string(&args, "format") {
            body["format"] = json!(format);
        }
        if let Some(links) = args.get("links").and_then(|v| v.as_bool()) {
            body["links"] = json!(links);
        }
        if let Some(image_links) = args.get("image_links").and_then(|v| v.as_bool()) {
            body["image_links"] = json!(image_links);
        }

        tracing::debug!(
            url_count = normalized_urls.len(),
            "[tinyfish_fetch] fetching URLs"
        );

        match self
            .client
            .post::<TinyFishFetchResponse>("/agent-integrations/tinyfish/fetch", &body)
            .await
        {
            Ok(resp) => {
                let mut lines = vec![format!(
                    "TinyFish fetched {} page(s), {} error(s).",
                    resp.results.len(),
                    resp.errors.len()
                )];

                for (idx, page) in resp.results.iter().enumerate() {
                    lines.push(format!(
                        "\n{}. {}",
                        idx + 1,
                        page.title.as_deref().unwrap_or(&page.url)
                    ));
                    lines.push(format!("   URL: {}", page.url));
                    if let Some(final_url) = page.final_url.as_deref() {
                        if final_url != page.url {
                            lines.push(format!("   Final URL: {final_url}"));
                        }
                    }
                    if let Some(description) = page.description.as_deref() {
                        if !description.is_empty() {
                            lines.push(format!("   Description: {description}"));
                        }
                    }
                    if let Some(language) = page.language.as_deref() {
                        lines.push(format!("   Language: {language}"));
                    }
                    if let Some(format) = page.format.as_deref() {
                        lines.push(format!("   Format: {format}"));
                    }
                    if let Some(latency_ms) = page.latency_ms {
                        lines.push(format!("   Latency: {latency_ms:.0}ms"));
                    }
                    if let Some(links) = page.links.as_ref() {
                        lines.push(format!("   Links: {}", links.len()));
                    }
                    if let Some(image_links) = page.image_links.as_ref() {
                        lines.push(format!("   Image links: {}", image_links.len()));
                    }

                    let text = match page.text.as_str() {
                        Some(s) => s.to_string(),
                        None => page.text.to_string(),
                    };
                    if !text.is_empty() && text != "null" {
                        let (snippet, truncated) = truncate_chars(&text, 1200);
                        lines.push("   Content:".to_string());
                        lines.push(snippet.to_string());
                        if truncated {
                            lines.push("   Content truncated to 1200 characters.".to_string());
                        }
                    }
                }

                if !resp.errors.is_empty() {
                    lines.push("\nErrors:".to_string());
                    for err in &resp.errors {
                        let status = err.status.map(|s| format!(" ({s})")).unwrap_or_default();
                        lines.push(format!("- {}: {}{}", err.url, err.error, status));
                    }
                }
                if let Some(cost_usd) = resp.cost_usd {
                    lines.push(format!("\nCost: ${cost_usd:.4}"));
                }

                Ok(ToolResult::success(lines.join("\n")))
            }
            Err(e) => {
                tracing::debug!(
                    url_count = normalized_urls.len(),
                    error = %e,
                    "[tinyfish_fetch] request failed"
                );
                Ok(ToolResult::error(format!("TinyFish fetch failed: {e}")))
            }
        }
    }
}

/// Run a TinyFish goal-based browser automation.
pub struct TinyFishAgentRunTool {
    client: Arc<IntegrationClient>,
}

impl TinyFishAgentRunTool {
    pub fn new(client: Arc<IntegrationClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for TinyFishAgentRunTool {
    fn name(&self) -> &str {
        "tinyfish_agent_run"
    }

    fn description(&self) -> &str {
        "Run a TinyFish goal-based browser automation on a target website. Provide a URL \
         and a specific natural-language goal with the desired output format."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "Target website URL to automate"
                },
                "goal": {
                    "type": "string",
                    "description": "Specific browser automation goal and expected output format"
                },
                "output_schema": {
                    "type": "object",
                    "description": "Optional structured-output JSON schema supported by TinyFish"
                },
                "browser_profile": {
                    "type": "string",
                    "enum": ["lite", "stealth"],
                    "description": "Browser profile for the run (default lite)",
                    "default": "lite"
                },
                "proxy_country_code": {
                    "type": "string",
                    "enum": ["US", "GB", "CA", "DE", "FR", "JP", "AU"],
                    "description": "Optional TinyFish proxy country code"
                },
                "use_vault": {
                    "type": "boolean",
                    "description": "Allow TinyFish vault credentials for this run when configured",
                    "default": false
                },
                "credential_item_ids": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional credential item IDs scoped to this run; requires use_vault=true"
                }
            },
            "required": ["url", "goal"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Workflow
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let url = non_empty_string(&args, "url")?;
        let goal = non_empty_string(&args, "goal")?;
        let mut body = json!({
            "url": url,
            "goal": goal,
        });

        if let Some(output_schema) = args.get("output_schema") {
            if !output_schema.is_object() {
                return Ok(ToolResult::error("output_schema must be a JSON object"));
            }
            body["output_schema"] = output_schema.clone();
        }
        if let Some(browser_profile) = optional_string(&args, "browser_profile") {
            body["browser_profile"] = json!(browser_profile);
        }
        if let Some(country_code) = optional_string(&args, "proxy_country_code") {
            body["proxy_config"] = json!({
                "enabled": true,
                "type": "tetra",
                "country_code": country_code,
            });
        }
        if let Some(use_vault) = args.get("use_vault").and_then(|v| v.as_bool()) {
            body["use_vault"] = json!(use_vault);
        }
        if let Some(ids) = args.get("credential_item_ids") {
            if !ids.is_array() {
                return Ok(ToolResult::error("credential_item_ids must be an array"));
            }
            body["credential_item_ids"] = ids.clone();
        }

        tracing::debug!(url = url, "[tinyfish_agent_run] starting automation");

        match self
            .client
            .post::<TinyFishAgentRunResponse>("/agent-integrations/tinyfish/agent/run", &body)
            .await
        {
            Ok(resp) => {
                tracing::debug!(
                    run_id = resp.run_id.as_deref().unwrap_or(""),
                    status = resp.status.as_str(),
                    has_error = resp.error.is_some(),
                    "[tinyfish_agent_run] request finished"
                );
                Ok(ToolResult::success(format_agent_run_response(resp)))
            }
            Err(e) => {
                tracing::debug!(
                    url = url,
                    error = %e,
                    "[tinyfish_agent_run] request failed"
                );
                Ok(ToolResult::error(format!(
                    "TinyFish automation failed: {e}"
                )))
            }
        }
    }
}

#[cfg(test)]
#[path = "tinyfish_tests.rs"]
mod tests;
