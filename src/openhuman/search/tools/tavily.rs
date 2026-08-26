//! Tavily Search + Extract integration — direct API (BYOK, not backend-proxied).
//!
//! **Scope**: Agent + CLI/RPC.
//!
//! **Endpoints**: `POST https://api.tavily.com/search`,
//! `POST https://api.tavily.com/extract`.
//!
//! **Auth**: `Authorization: Bearer <api key>`.
//!
//! When the user selects `tavily` as their search engine and has saved their own
//! Tavily API key, every call in this family goes straight from the desktop
//! client to `api.tavily.com` — the OpenHuman managed backend is never
//! involved. The managed (`engine = "managed"`) path is untouched by this module.

use crate::openhuman::tools::traits::{Tool, ToolCallOptions, ToolResult};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;

const DEFAULT_API_URL: &str = "https://api.tavily.com";
const SEARCH_EXCERPT_MAX_CHARS: usize = 500;
const SEARCH_RAW_CONTENT_MAX_CHARS: usize = 8_000;
const MAX_QUERY_IMAGES: usize = 10;
const MAX_IMAGES_PER_RESULT: usize = 3;
const IMAGE_DESCRIPTION_MAX_CHARS: usize = 300;

/// Tavily may return image entries as a bare URL or as an object with an
/// optional description, depending on the endpoint options and API version.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum TavilyImage {
    Url(String),
    Detailed {
        #[serde(default)]
        url: String,
        #[serde(default)]
        description: Option<String>,
    },
}

impl TavilyImage {
    fn url(&self) -> &str {
        match self {
            Self::Url(url) | Self::Detailed { url, .. } => url,
        }
    }

    fn description(&self) -> Option<String> {
        let description = match self {
            Self::Url(_) => None,
            Self::Detailed { description, .. } => non_empty(description.as_deref()),
        }?;
        let single_line = description.replace(['\r', '\n'], " ");
        Some(crate::openhuman::util::truncate_with_ellipsis(
            &single_line,
            IMAGE_DESCRIPTION_MAX_CHARS,
        ))
    }
}

/// One Tavily search result, shared by `/search`.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct TavilyResultItem {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub score: Option<f64>,
    #[serde(default, rename = "raw_content")]
    pub raw_content: Option<String>,
    #[serde(default)]
    pub images: Vec<TavilyImage>,
}

impl TavilyResultItem {
    /// Best available excerpt: the chunked `content` first, then the full
    /// cleaned page (`raw_content`), only present when `include_raw_content`
    /// was requested.
    fn excerpt(&self) -> Option<String> {
        non_empty(self.content.as_deref()).or_else(|| non_empty(self.raw_content.as_deref()))
    }

    fn display_title(&self) -> &str {
        self.title
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .unwrap_or("Untitled")
    }
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct TavilySearchResponse {
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub answer: Option<String>,
    #[serde(default)]
    pub images: Vec<TavilyImage>,
    #[serde(default)]
    pub results: Vec<TavilyResultItem>,
}

/// One URL's extracted content from `/extract`.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct TavilyExtractResult {
    #[serde(default)]
    pub url: String,
    #[serde(default, rename = "raw_content")]
    pub raw_content: Option<String>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct TavilyExtractResponse {
    #[serde(default)]
    pub results: Vec<TavilyExtractResult>,
    #[serde(default, rename = "failed_results")]
    pub failed_results: Vec<TavilyExtractFailure>,
}

/// A URL Tavily could not process. Errors are rendered but never attributed to
/// a template — the `error` string is remote-controlled and must not be trusted.
#[derive(Debug, Default, Deserialize, Serialize)]
pub struct TavilyExtractFailure {
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

fn non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Escape a remote page title for use as a markdown link label. Titles are
/// attacker-controlled, and an unescaped `[`/`]` breaks out of the link and
/// lets a crafted page inject markdown into the agent transcript.
fn escape_link_text(raw: &str) -> String {
    raw.replace('\\', r"\\")
        .replace('[', r"\[")
        .replace(']', r"\]")
}

/// Render a URL as a markdown link destination. Bare parentheses (common in
/// Wikipedia URLs) terminate the destination early, so wrap in angle brackets
/// and drop the characters that would close them.
fn escape_link_destination(raw: &str) -> String {
    let cleaned: String = raw
        .trim()
        .chars()
        .filter(|c| !matches!(c, '<' | '>' | ' '))
        .collect();
    format!("<{cleaned}>")
}

/// Shared HTTP plumbing for the Tavily BYOK tool family. Holds the user's key and
/// the direct `api.tavily.com` base URL (overridable in tests).
#[derive(Clone)]
pub(crate) struct TavilyClient {
    api_key: Option<String>,
    api_url: String,
    max_results: usize,
    timeout_secs: u64,
}

impl TavilyClient {
    pub(crate) fn new(
        api_key: Option<String>,
        api_url: Option<String>,
        max_results: usize,
        timeout_secs: u64,
    ) -> Self {
        Self {
            api_key,
            api_url: api_url.unwrap_or_else(|| DEFAULT_API_URL.to_string()),
            max_results: max_results.clamp(1, 20),
            timeout_secs: timeout_secs.max(1),
        }
    }

    /// Build the HTTP client at call time, like `brave.rs::http_client`, so a
    /// TLS-backend failure surfaces as a tool error instead of aborting the
    /// process while a session is being constructed.
    fn http_client(&self) -> anyhow::Result<reqwest::Client> {
        crate::openhuman::util::tls::tls_client_builder()
            .timeout(Duration::from_secs(self.timeout_secs))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| anyhow::anyhow!("failed to build Tavily HTTP client: {e}"))
    }

    /// Destination host for the egress descriptor, e.g. `api.tavily.com`.
    fn egress_host(&self) -> String {
        reqwest::Url::parse(&self.api_url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_string))
            .unwrap_or_else(|| "api.tavily.com".to_string())
    }

    /// Egress descriptor for a call to Tavily. The query (or the requested URLs)
    /// is user content leaving the device, so it carries `Prompt` on top of the
    /// destination `Url` that `network_fetch` supplies.
    fn egress_descriptor(&self) -> crate::openhuman::security::egress::EgressDescriptor {
        crate::openhuman::security::egress::EgressDescriptor::network_fetch(self.egress_host())
            .with_data_kind(crate::openhuman::security::egress::DataKind::Prompt)
    }

    /// Privacy epic S7 (#4441): under `LocalOnly` the search is refused before
    /// anything reaches Tavily. Returns the `[policy-blocked]` tool result to hand
    /// straight back from `execute`, or `None` when the transfer is permitted.
    fn local_only_block(&self) -> Option<ToolResult> {
        crate::openhuman::security::egress::local_only_tool_block(&self.egress_descriptor())
            .map(ToolResult::error)
    }

    /// The configured key, or a user-actionable error naming exactly where to
    /// set it. Surfaced verbatim on the first search attempt.
    fn key(&self) -> anyhow::Result<&str> {
        self.api_key
            .as_deref()
            .map(str::trim)
            .filter(|k| !k.is_empty())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Tavily search unavailable: no API key configured. Add your Tavily API key \
                     under Connections > Search engine, set TAVILY_API_KEY or \
                     OPENHUMAN_TAVILY_API_KEY, or add search.tavily.api_key to config.toml."
                )
            })
    }

    /// Requested result count, honouring both `max_results` and Tavily's native
    /// `max_results` spelling. An explicit per-call value is clamped to the
    /// API's own 1..=20 range rather than to the configured `max_results` --
    /// config supplies the *default* when the call omits one, and a caller may
    /// ask for more (this matches `querit.rs`).
    fn requested_results(&self, args: &Value) -> usize {
        args.get("max_results")
            .and_then(Value::as_u64)
            .map(|n| n.clamp(1, 20) as usize)
            .unwrap_or(self.max_results)
    }

    async fn post(&self, path: &str, body: Value) -> anyhow::Result<Value> {
        let api_key = self.key()?;
        let client = self.http_client()?;
        let url = format!("{}/{}", self.api_url.trim_end_matches('/'), path);
        tracing::debug!(
            path,
            timeout_secs = self.timeout_secs,
            "[tavily] POST {url} (direct BYOK)"
        );

        // Egress spine (privacy epic S2, #4436): disclose the destination before
        // contacting Tavily. `local_only_block` has already refused the call if the
        // live policy forbids it.
        crate::openhuman::security::egress::emit_external_transfer(self.egress_descriptor());

        let resp = client
            .post(&url)
            .bearer_auth(api_key)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                tracing::warn!("[tavily] request to {path} failed: {e}");
                anyhow::anyhow!("Tavily request failed: {e}")
            })?;

        let status = resp.status();
        if !status.is_success() {
            // Read and drop the body: Tavily echoes the query back in errors and
            // the message could reach the agent transcript.
            let body_len = resp.text().await.unwrap_or_default().len();
            tracing::warn!(status = %status, body_len, "[tavily] non-2xx response");
            if status == reqwest::StatusCode::UNAUTHORIZED
                || status == reqwest::StatusCode::FORBIDDEN
            {
                anyhow::bail!(
                    "Tavily rejected the configured API key (HTTP {status}). \
                     Check your Tavily API key under Connections > Search engine."
                );
            }
            anyhow::bail!("Tavily returned non-2xx status {status}");
        }

        resp.json().await.map_err(|e| {
            tracing::warn!("[tavily] failed to read response JSON: {e}");
            anyhow::anyhow!("Failed to read Tavily response JSON: {e}")
        })
    }

    fn render_plain(
        &self,
        results: &[TavilyResultItem],
        query_images: &[TavilyImage],
        heading: &str,
        limit: usize,
        include_raw_content: bool,
        include_images: bool,
    ) -> String {
        let mut lines = if results.is_empty() {
            vec![format!("No Tavily results for: {heading} (via Tavily)")]
        } else {
            vec![format!("Search results for: {heading} (via Tavily)")]
        };
        for (i, item) in results.iter().take(limit).enumerate() {
            lines.push(format!("{}. {}", i + 1, item.display_title()));
            lines.push(format!("   {}", item.url.trim()));
            if let Some(score) = item.score {
                lines.push(format!("   Score: {score:.3}"));
            }
            if let Some(excerpt) = item.excerpt() {
                let truncated = crate::openhuman::util::truncate_with_ellipsis(
                    &excerpt,
                    SEARCH_EXCERPT_MAX_CHARS,
                );
                lines.push(format!("   {truncated}"));
            }
            if include_raw_content {
                if let Some(raw_content) = non_empty(item.raw_content.as_deref()) {
                    let truncated = crate::openhuman::util::truncate_with_ellipsis(
                        &raw_content,
                        SEARCH_RAW_CONTENT_MAX_CHARS,
                    );
                    lines.push(format!("   Full content:\n{truncated}"));
                }
            }
            if include_images {
                for image in item.images.iter().take(MAX_IMAGES_PER_RESULT) {
                    let url = image.url().trim();
                    if url.is_empty() {
                        continue;
                    }
                    let description = image
                        .description()
                        .map(|value| format!(" — {value}"))
                        .unwrap_or_default();
                    lines.push(format!("   Image: {url}{description}"));
                }
            }
        }
        if include_images && !query_images.is_empty() {
            lines.push("Query-related images:".to_string());
            for image in query_images.iter().take(MAX_QUERY_IMAGES) {
                let url = image.url().trim();
                if url.is_empty() {
                    continue;
                }
                let description = image
                    .description()
                    .map(|value| format!(" — {value}"))
                    .unwrap_or_default();
                lines.push(format!("- {url}{description}"));
            }
        }
        lines.join("\n")
    }

    fn render_markdown(
        &self,
        results: &[TavilyResultItem],
        query_images: &[TavilyImage],
        heading: &str,
        limit: usize,
        answer: Option<&str>,
        include_raw_content: bool,
        include_images: bool,
    ) -> String {
        let mut out = if results.is_empty() {
            format!("_No results for `{heading}`_ (via Tavily)\n")
        } else {
            format!("# Search results -- `{heading}` (via Tavily)\n")
        };
        if let Some(answer) = answer {
            out.push_str("\n## Answer\n\n");
            out.push_str(answer);
            out.push('\n');
        }
        for item in results.iter().take(limit) {
            out.push_str(&format!(
                "\n## [{}]({})\n",
                escape_link_text(item.display_title()),
                escape_link_destination(&item.url)
            ));
            if let Some(excerpt) = item.excerpt() {
                let truncated = crate::openhuman::util::truncate_with_suffix(
                    &excerpt,
                    SEARCH_EXCERPT_MAX_CHARS,
                    "...",
                );
                out.push_str(&format!("> {truncated}\n"));
            }
            if include_raw_content {
                if let Some(raw_content) = non_empty(item.raw_content.as_deref()) {
                    let truncated = crate::openhuman::util::truncate_with_suffix(
                        &raw_content,
                        SEARCH_RAW_CONTENT_MAX_CHARS,
                        "...",
                    );
                    out.push_str("\n### Full content\n\n");
                    out.push_str(&truncated);
                    out.push('\n');
                }
            }
            if include_images {
                for image in item.images.iter().take(MAX_IMAGES_PER_RESULT) {
                    let url = image.url().trim();
                    if url.is_empty() {
                        continue;
                    }
                    let description = image
                        .description()
                        .map(|value| format!(" — {value}"))
                        .unwrap_or_default();
                    out.push_str(&format!(
                        "\n[Image]({}){description}\n",
                        escape_link_destination(url)
                    ));
                }
            }
        }
        if include_images && !query_images.is_empty() {
            out.push_str("\n## Query-related images\n");
            for image in query_images.iter().take(MAX_QUERY_IMAGES) {
                let url = image.url().trim();
                if url.is_empty() {
                    continue;
                }
                let description = image
                    .description()
                    .map(|value| format!(" — {value}"))
                    .unwrap_or_default();
                out.push_str(&format!(
                    "\n- [Image]({}){description}",
                    escape_link_destination(url)
                ));
            }
            out.push('\n');
        }
        out
    }
}

/// Copy a string array argument onto the Tavily request body.
fn copy_domain_filter(args: &Value, from: &str, body: &mut Value) {
    if let Some(list) = args.get(from).filter(|v| v.is_array()) {
        body[from] = list.clone();
    }
}

/// Copy an optional string argument onto the Tavily request body under the
/// same (already snake_case) key.
fn copy_string(args: &Value, key: &str, body: &mut Value) {
    if let Some(value) = non_empty(args.get(key).and_then(Value::as_str)) {
        body[key] = json!(value);
    }
}

/// Copy an optional boolean argument onto the Tavily request body.
fn copy_bool(args: &Value, key: &str, body: &mut Value) {
    if let Some(value) = args.get(key).and_then(Value::as_bool) {
        body[key] = json!(value);
    }
}

/// Web / news / finance search via the Tavily API (`POST /search`).
pub struct TavilySearchTool {
    tool_name: &'static str,
    client: TavilyClient,
}

impl TavilySearchTool {
    pub fn new(
        api_key: Option<String>,
        api_url: Option<String>,
        max_results: usize,
        timeout_secs: u64,
    ) -> Self {
        Self {
            tool_name: "tavily_search",
            client: TavilyClient::new(api_key, api_url, max_results, timeout_secs),
        }
    }

    /// Same tool under the canonical `web_search_tool` slot, so selecting Tavily
    /// satisfies the agent's generic "search the web" affordance.
    pub fn new_web_search_tool(
        api_key: Option<String>,
        api_url: Option<String>,
        max_results: usize,
        timeout_secs: u64,
    ) -> Self {
        Self {
            tool_name: "web_search_tool",
            client: TavilyClient::new(api_key, api_url, max_results, timeout_secs),
        }
    }

    fn build_body(&self, args: &Value, query: &str) -> Value {
        let mut body = json!({
            "query": query,
            "max_results": self.client.requested_results(args),
        });
        copy_string(args, "search_depth", &mut body);
        copy_string(args, "topic", &mut body);
        copy_string(args, "time_range", &mut body);
        copy_string(args, "start_date", &mut body);
        copy_string(args, "end_date", &mut body);
        copy_bool(args, "include_answer", &mut body);
        copy_bool(args, "include_raw_content", &mut body);
        copy_bool(args, "include_images", &mut body);
        copy_domain_filter(args, "include_domains", &mut body);
        copy_domain_filter(args, "exclude_domains", &mut body);
        body
    }
}

#[async_trait]
impl Tool for TavilySearchTool {
    fn name(&self) -> &str {
        self.tool_name
    }

    fn description(&self) -> &str {
        "Search the web with Tavily. Returns ranked pages with URLs, titles, and \
         snippets. Supports general, news, and finance topics; search-depth levels; \
         a publish/update time range or explicit start/end dates; domain include/ \
         exclude filters; and an optional LLM-generated answer, cleaned page content, \
         or image links."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query. Write it in the language you want results in."
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum number of results to return (default from config, max 20)."
                },
                "search_depth": {
                    "type": "string",
                    "enum": ["basic", "advanced", "fast", "ultra-fast"],
                    "description": "Latency vs. relevance tradeoff. Advanced costs more and gives higher-relevance results; ultra-fast is the quickest."
                },
                "topic": {
                    "type": "string",
                    "enum": ["general", "news", "finance"],
                    "description": "Category of the search. Use 'news' for real-time coverage, 'finance' for financial data."
                },
                "time_range": {
                    "type": "string",
                    "enum": ["day", "week", "month", "year"],
                    "description": "Only results published or updated within this period back from today. Alternative to start_date/end_date."
                },
                "start_date": {
                    "type": "string",
                    "description": "Only results published/updated on or after this YYYY-MM-DD date."
                },
                "end_date": {
                    "type": "string",
                    "description": "Only results published/updated on or before this YYYY-MM-DD date."
                },
                "include_answer": {
                    "type": "boolean",
                    "description": "Also return an LLM-generated concise answer to the query."
                },
                "include_raw_content": {
                    "type": "boolean",
                    "description": "Also return cleaned page content for each result, bounded by OpenHuman to protect the agent context (larger responses)."
                },
                "include_images": {
                    "type": "boolean",
                    "description": "Also return query-related images in the response."
                },
                "include_domains": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Only return results from these domains."
                },
                "exclude_domains": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Exclude results from these domains."
                }
            },
            "required": ["query"]
        })
    }

    fn supports_markdown(&self) -> bool {
        true
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        self.execute_with_options(args, ToolCallOptions::default())
            .await
    }

    async fn execute_with_options(
        &self,
        args: Value,
        options: ToolCallOptions,
    ) -> anyhow::Result<ToolResult> {
        if let Some(blocked) = self.client.local_only_block() {
            return Ok(blocked);
        }

        let query = non_empty(args.get("query").and_then(Value::as_str))
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: query"))?;

        let limit = self.client.requested_results(&args);
        let body = self.build_body(&args, &query);
        let value = self.client.post("search", body).await?;
        let parsed: TavilySearchResponse = serde_json::from_value(value).map_err(|e| {
            tracing::warn!("[tavily] failed to parse search response: {e}");
            anyhow::anyhow!("Failed to parse Tavily search response: {e}")
        })?;

        tracing::debug!(result_count = parsed.results.len(), "[tavily] search ok");

        // A Tavily-generated answer is the provider's own synthesis of the
        // results; surface it when requested so the agent does not re-answer
        // from the scraps.
        let answer = non_empty(parsed.answer.as_deref());
        let include_raw_content = args
            .get("include_raw_content")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let include_images = args
            .get("include_images")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let plain = self.client.render_plain(
            &parsed.results,
            &parsed.images,
            &query,
            limit,
            include_raw_content,
            include_images,
        );
        let markdown = self.client.render_markdown(
            &parsed.results,
            &parsed.images,
            &query,
            limit,
            answer.as_deref(),
            include_raw_content,
            include_images,
        );
        let output = match answer {
            Some(answer) => format!("Answer: {answer}\n\n{plain}"),
            None => plain,
        };

        let mut result = ToolResult::success(output);
        if options.prefer_markdown {
            result.markdown_formatted = Some(markdown);
        }
        Ok(result)
    }
}

/// Retrieve full page contents for a list of URLs (`POST /extract`).
pub struct TavilyExtractTool {
    client: TavilyClient,
}

impl TavilyExtractTool {
    pub fn new(
        api_key: Option<String>,
        api_url: Option<String>,
        max_results: usize,
        timeout_secs: u64,
    ) -> Self {
        // Extraction transfers the full cleaned page, which can be hundreds of
        // kilobytes or more over a slow link, while search only moves small
        // excerpts. A hot folder of small search responses needs 15s; a single
        // large page commonly does not. Tavily itself allows up to 60s per
        // request, so the extract budget must never be smaller than that cap —
        // otherwise a large page dies to `operation timed out` after the server
        // already answered 200.
        Self {
            client: TavilyClient::new(api_key, api_url, max_results, timeout_secs.max(60)),
        }
    }

    /// Accept either a `urls` array or a single `url` string, so the agent can
    /// call this the obvious way for the one-document case.
    fn collect_urls(args: &Value) -> anyhow::Result<Vec<String>> {
        let urls: Vec<String> = match args.get("urls") {
            Some(Value::Array(items)) => items
                .iter()
                .filter_map(|v| non_empty(v.as_str()))
                .collect::<Vec<_>>(),
            Some(Value::String(single)) => non_empty(Some(single)).into_iter().collect(),
            _ => non_empty(args.get("url").and_then(Value::as_str))
                .into_iter()
                .collect(),
        };
        if urls.is_empty() {
            anyhow::bail!("Missing required parameter: urls (a non-empty list of page URLs)");
        }
        if urls.len() > 20 {
            anyhow::bail!(
                "Tavily extract accepts at most 20 URLs per call (got {})",
                urls.len()
            );
        }
        Ok(urls)
    }

    fn render_plain(&self, results: &[TavilyExtractResult]) -> String {
        if results.is_empty() {
            return "No Tavily extraction results.".to_string();
        }
        let mut lines = Vec::new();
        for (i, item) in results.iter().enumerate() {
            lines.push(format!("{}. {}", i + 1, item.url.trim()));
            match non_empty(item.raw_content.as_deref()) {
                Some(content) => {
                    let truncated = crate::openhuman::util::truncate_with_ellipsis(&content, 8_000);
                    lines.push(format!("   {truncated}"));
                }
                None => lines.push("   (no content extracted)".to_string()),
            }
        }
        lines.join("\n")
    }

    fn render_markdown(&self, results: &[TavilyExtractResult]) -> String {
        if results.is_empty() {
            return "_No Tavily extraction results._".to_string();
        }
        let mut out = String::from("# Tavily extraction\n");
        for item in results {
            out.push_str(&format!(
                "\n## [{}]({})\n",
                escape_link_text(item.url.trim()),
                escape_link_destination(&item.url)
            ));
            match non_empty(item.raw_content.as_deref()) {
                Some(content) => {
                    let truncated =
                        crate::openhuman::util::truncate_with_suffix(&content, 8_000, "...");
                    out.push_str(&truncated);
                    out.push('\n');
                }
                None => out.push_str("_No content extracted._\n"),
            }
        }
        out
    }
}

#[async_trait]
impl Tool for TavilyExtractTool {
    fn name(&self) -> &str {
        "tavily_extract"
    }

    fn description(&self) -> &str {
        "Extract cleaned content from one or more web pages using Tavily. Returns \
         markdown or plain text, bounded to 8,000 characters per URL."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "urls": {
                    "type": "array",
                    "items": { "type": "string" },
                    "minItems": 1,
                    "maxItems": 20,
                    "description": "The page URLs to extract content from (up to 20)."
                },
                "format": {
                    "type": "string",
                    "enum": ["markdown", "text"],
                    "description": "Format of the extracted content. Defaults to markdown."
                },
                "extract_depth": {
                    "type": "string",
                    "enum": ["basic", "advanced"],
                    "description": "basic is faster and cheaper; advanced also captures tables and embedded content."
                }
            },
            "required": ["urls"]
        })
    }

    fn supports_markdown(&self) -> bool {
        true
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        self.execute_with_options(args, ToolCallOptions::default())
            .await
    }

    async fn execute_with_options(
        &self,
        args: Value,
        options: ToolCallOptions,
    ) -> anyhow::Result<ToolResult> {
        if let Some(blocked) = self.client.local_only_block() {
            return Ok(blocked);
        }

        let urls = Self::collect_urls(&args)?;

        let mut body = json!({
            "urls": urls,
            "format": "markdown",
        });
        copy_string(&args, "format", &mut body);
        copy_string(&args, "extract_depth", &mut body);

        let value = self.client.post("extract", body).await?;
        let parsed: TavilyExtractResponse = serde_json::from_value(value).map_err(|e| {
            tracing::warn!("[tavily] failed to parse extract response: {e}");
            anyhow::anyhow!("Failed to parse Tavily extract response: {e}")
        })?;

        tracing::debug!(result_count = parsed.results.len(), "[tavily] extract ok");

        let failed = parsed.failed_results.len();
        if failed > 0 {
            tracing::warn!(failed, "[tavily] extraction failures");
        }
        if parsed.results.is_empty() && failed > 0 {
            return Ok(ToolResult::error(format!(
                "Tavily could not extract any of the requested URLs ({failed} failed)."
            )));
        }

        let plain = self.render_plain(&parsed.results);
        let mut markdown = self.render_markdown(&parsed.results);
        let output = if failed == 0 {
            plain
        } else {
            // Report failures as a trailing line without echoing the remote
            // error verbatim into the transcript.
            markdown.push_str(&format!(
                "\n> {failed} URL(s) could not be extracted by Tavily.\n"
            ));
            format!("{plain}\n\n({failed} URL(s) could not be extracted by Tavily)")
        };

        let mut result = ToolResult::success(output);
        if options.prefer_markdown {
            result.markdown_formatted = Some(markdown);
        }
        Ok(result)
    }
}

#[cfg(test)]
#[path = "tavily_tests.rs"]
mod tests;
