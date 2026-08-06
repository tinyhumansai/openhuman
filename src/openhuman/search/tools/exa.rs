//! Exa neural search integration -- direct API (BYOK, not backend-proxied).
//!
//! **Scope**: Agent + CLI/RPC.
//!
//! **Endpoints**: `POST https://api.exa.ai/search`,
//! `POST https://api.exa.ai/findSimilar`, `POST https://api.exa.ai/contents`.
//!
//! **Auth**: `x-api-key: <api key>`.
//!
//! When the user selects `exa` as their search engine and has saved their own
//! Exa API key, every call in this family goes straight from the desktop client
//! to `api.exa.ai` -- the OpenHuman managed backend is never involved. The
//! managed (`engine = "managed"`) path is untouched by this module.

use crate::openhuman::tools::traits::{Tool, ToolCallOptions, ToolResult};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;

const DEFAULT_API_URL: &str = "https://api.exa.ai";

/// One Exa document, shared by `/search`, `/findSimilar`, and `/contents`.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ExaResultItem {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default, rename = "publishedDate")]
    pub published_date: Option<String>,
    #[serde(default)]
    pub score: Option<f64>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub highlights: Vec<String>,
}

impl ExaResultItem {
    /// Best available excerpt: explicit summary, then highlights, then the
    /// crawled page text.
    fn excerpt(&self) -> Option<String> {
        non_empty(self.summary.as_deref())
            .or_else(|| {
                let joined = self
                    .highlights
                    .iter()
                    .map(|h| h.trim())
                    .filter(|h| !h.is_empty())
                    .collect::<Vec<_>>()
                    .join(" ");
                non_empty(Some(joined.as_str()))
            })
            .or_else(|| non_empty(self.text.as_deref()))
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
pub struct ExaSearchResponse {
    #[serde(default)]
    pub results: Vec<ExaResultItem>,
    #[serde(default, rename = "autopromptString")]
    pub autoprompt_string: Option<String>,
    #[serde(default, rename = "resolvedSearchType")]
    pub resolved_search_type: Option<String>,
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

/// Shared HTTP plumbing for the Exa BYOK tool family. Holds the user's key and
/// the direct `api.exa.ai` base URL (overridable in tests).
#[derive(Clone)]
pub(crate) struct ExaClient {
    api_key: Option<String>,
    api_url: String,
    max_results: usize,
    timeout_secs: u64,
}

impl ExaClient {
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
            .map_err(|e| anyhow::anyhow!("failed to build Exa HTTP client: {e}"))
    }

    /// Destination host for the egress descriptor, e.g. `api.exa.ai`.
    fn egress_host(&self) -> String {
        reqwest::Url::parse(&self.api_url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_string))
            .unwrap_or_else(|| "api.exa.ai".to_string())
    }

    /// Egress descriptor for a call to Exa. The query (or the requested URLs)
    /// is user content leaving the device, so it carries `Prompt` on top of the
    /// destination `Url` that `network_fetch` supplies.
    fn egress_descriptor(&self) -> crate::openhuman::security::egress::EgressDescriptor {
        crate::openhuman::security::egress::EgressDescriptor::network_fetch(self.egress_host())
            .with_data_kind(crate::openhuman::security::egress::DataKind::Prompt)
    }

    /// Privacy epic S7 (#4441): under `LocalOnly` the search is refused before
    /// anything reaches Exa. Returns the `[policy-blocked]` tool result to hand
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
                    "Exa search unavailable: no API key configured. Add your Exa API key \
                     under Connections > Search engine, set EXA_API_KEY or \
                     OPENHUMAN_EXA_API_KEY, or add search.exa.api_key to config.toml."
                )
            })
    }

    /// Requested result count, honouring both `max_results` and Exa's native
    /// `numResults` spelling. An explicit per-call value is clamped to the
    /// API's own 1..=20 range rather than to the configured `max_results` --
    /// config supplies the *default* when the call omits one, and a caller may
    /// ask for more (this matches `querit.rs`).
    fn requested_results(&self, args: &Value) -> usize {
        args.get("max_results")
            .or_else(|| args.get("num_results"))
            .or_else(|| args.get("numResults"))
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
            "[exa] POST {url} (direct BYOK)"
        );

        // Egress spine (privacy epic S2, #4436): disclose the destination before
        // contacting Exa. `local_only_block` has already refused the call if the
        // live policy forbids it.
        crate::openhuman::security::egress::emit_external_transfer(self.egress_descriptor());

        let resp = client
            .post(&url)
            .header("x-api-key", api_key)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                tracing::warn!("[exa] request to {path} failed: {e}");
                anyhow::anyhow!("Exa request failed: {e}")
            })?;

        let status = resp.status();
        if !status.is_success() {
            // Read and drop the body: Exa echoes the query back in errors and
            // the message reaches the agent transcript.
            let body_len = resp.text().await.unwrap_or_default().len();
            tracing::warn!(status = %status, body_len, "[exa] non-2xx response");
            if status == reqwest::StatusCode::UNAUTHORIZED
                || status == reqwest::StatusCode::FORBIDDEN
            {
                anyhow::bail!(
                    "Exa rejected the configured API key (HTTP {status}). \
                     Check your Exa API key under Connections > Search engine."
                );
            }
            anyhow::bail!("Exa returned non-2xx status {status}");
        }

        resp.json().await.map_err(|e| {
            tracing::warn!("[exa] failed to read response JSON: {e}");
            anyhow::anyhow!("Failed to read Exa response JSON: {e}")
        })
    }

    async fn post_documents(&self, path: &str, body: Value) -> anyhow::Result<Vec<ExaResultItem>> {
        let value = self.post(path, body).await?;
        let parsed: ExaSearchResponse = serde_json::from_value(value).map_err(|e| {
            tracing::warn!("[exa] failed to parse {path} response: {e}");
            anyhow::anyhow!("Failed to parse Exa response: {e}")
        })?;
        tracing::debug!(path, result_count = parsed.results.len(), "[exa] call ok");
        Ok(parsed.results)
    }

    fn render_plain(&self, results: &[ExaResultItem], heading: &str, limit: usize) -> String {
        if results.is_empty() {
            return format!("No Exa results for: {heading}");
        }

        let mut lines = vec![format!("Search results for: {heading} (via Exa)")];
        for (i, item) in results.iter().take(limit).enumerate() {
            lines.push(format!("{}. {}", i + 1, item.display_title()));
            lines.push(format!("   {}", item.url.trim()));
            if let Some(date) = non_empty(item.published_date.as_deref()) {
                lines.push(format!("   Published: {date}"));
            }
            if let Some(author) = non_empty(item.author.as_deref()) {
                lines.push(format!("   Author: {author}"));
            }
            if let Some(excerpt) = item.excerpt() {
                let truncated = crate::openhuman::util::truncate_with_ellipsis(&excerpt, 500);
                lines.push(format!("   {truncated}"));
            }
        }
        lines.join("\n")
    }

    fn render_markdown(&self, results: &[ExaResultItem], heading: &str, limit: usize) -> String {
        if results.is_empty() {
            return format!("_No Exa results for `{heading}`._");
        }

        let mut out = format!("# Exa results -- `{heading}`\n");
        for item in results.iter().take(limit) {
            out.push_str(&format!(
                "\n## [{}]({})\n",
                escape_link_text(item.display_title()),
                escape_link_destination(&item.url)
            ));
            if let Some(date) = non_empty(item.published_date.as_deref()) {
                out.push_str(&format!("_Published: {date}_\n\n"));
            }
            if let Some(author) = non_empty(item.author.as_deref()) {
                out.push_str(&format!("_Author: {author}_\n\n"));
            }
            if let Some(excerpt) = item.excerpt() {
                let truncated = crate::openhuman::util::truncate_with_suffix(&excerpt, 500, "...");
                out.push_str(&format!("> {truncated}\n"));
            }
        }
        out
    }

    /// `limit` caps how many documents are rendered. Search and find-similar
    /// pass the requested result count; `exa_get_contents` passes the URL count
    /// so a large batch is never silently trimmed to `max_results`.
    fn to_result(
        &self,
        results: &[ExaResultItem],
        heading: &str,
        limit: usize,
        options: &ToolCallOptions,
    ) -> ToolResult {
        let mut result = ToolResult::success(self.render_plain(results, heading, limit));
        if options.prefer_markdown {
            result.markdown_formatted = Some(self.render_markdown(results, heading, limit));
        }
        result
    }
}

/// Copy a string array argument onto the Exa request body under its native key.
fn copy_domain_filter(args: &Value, from: &str, to: &str, body: &mut Value) {
    if let Some(list) = args.get(from).filter(|v| v.is_array()) {
        body[to] = list.clone();
    }
}

/// `contents` sub-object shared by search and find-similar. Exa only crawls
/// page text when asked, so this stays opt-in to keep responses small.
fn contents_request(args: &Value) -> Option<Value> {
    let want_text = args
        .get("include_text")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let want_highlights = args
        .get("include_highlights")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !want_text && !want_highlights {
        return None;
    }
    let mut contents = json!({});
    if want_text {
        contents["text"] = json!(true);
    }
    if want_highlights {
        contents["highlights"] = json!(true);
    }
    Some(contents)
}

/// Neural / keyword web search via the Exa API (`POST /search`).
pub struct ExaSearchTool {
    tool_name: &'static str,
    client: ExaClient,
}

impl ExaSearchTool {
    pub fn new(
        api_key: Option<String>,
        api_url: Option<String>,
        max_results: usize,
        timeout_secs: u64,
    ) -> Self {
        Self {
            tool_name: "exa_search",
            client: ExaClient::new(api_key, api_url, max_results, timeout_secs),
        }
    }

    /// Same tool under the canonical `web_search_tool` slot, so selecting Exa
    /// satisfies the agent's generic "search the web" affordance.
    pub fn new_web_search_tool(
        api_key: Option<String>,
        api_url: Option<String>,
        max_results: usize,
        timeout_secs: u64,
    ) -> Self {
        Self {
            tool_name: "web_search_tool",
            client: ExaClient::new(api_key, api_url, max_results, timeout_secs),
        }
    }

    fn build_body(&self, args: &Value, query: &str) -> Value {
        let mut body = json!({
            "query": query,
            "numResults": self.client.requested_results(args),
        });
        if let Some(search_type) = non_empty(args.get("type").and_then(Value::as_str)) {
            body["type"] = json!(search_type);
        }
        if let Some(category) = non_empty(args.get("category").and_then(Value::as_str)) {
            body["category"] = json!(category);
        }
        copy_domain_filter(args, "include_domains", "includeDomains", &mut body);
        copy_domain_filter(args, "exclude_domains", "excludeDomains", &mut body);
        if let Some(start) = non_empty(args.get("start_published_date").and_then(Value::as_str)) {
            body["startPublishedDate"] = json!(start);
        }
        if let Some(end) = non_empty(args.get("end_published_date").and_then(Value::as_str)) {
            body["endPublishedDate"] = json!(end);
        }
        if let Some(contents) = contents_request(args) {
            body["contents"] = contents;
        }
        body
    }
}

#[async_trait]
impl Tool for ExaSearchTool {
    fn name(&self) -> &str {
        self.tool_name
    }

    fn description(&self) -> &str {
        "Search the web with Exa. Returns ranked pages with URLs, titles, publish dates, \
         and optional page text. Supports search modes from instant to deep-reasoning, \
         domain include/exclude filters, a published-date range, and result categories."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query. Natural-language phrasing works well with neural search."
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum number of results to return (default from config, max 20)."
                },
                "type": {
                    "type": "string",
                    "enum": ["auto", "instant", "fast", "deep-lite", "deep", "deep-reasoning"],
                    "description": "Exa search mode, fastest to most thorough. Defaults to Exa's own 'auto' selection."
                },
                "category": {
                    "type": "string",
                    "description": "Restrict to an Exa category, e.g. 'company', 'research paper', 'news', 'financial report', 'personal site'."
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
                },
                "start_published_date": {
                    "type": "string",
                    "description": "Only results published on or after this ISO-8601 date."
                },
                "end_published_date": {
                    "type": "string",
                    "description": "Only results published on or before this ISO-8601 date."
                },
                "include_text": {
                    "type": "boolean",
                    "description": "Also crawl and return the page text for each result (slower)."
                },
                "include_highlights": {
                    "type": "boolean",
                    "description": "Also return query-relevant highlight snippets for each result."
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
        let results = self.client.post_documents("search", body).await?;
        Ok(self.client.to_result(&results, &query, limit, &options))
    }
}

/// Find pages similar to a URL via the Exa API (`POST /findSimilar`).
pub struct ExaFindSimilarTool {
    client: ExaClient,
}

impl ExaFindSimilarTool {
    pub fn new(
        api_key: Option<String>,
        api_url: Option<String>,
        max_results: usize,
        timeout_secs: u64,
    ) -> Self {
        Self {
            client: ExaClient::new(api_key, api_url, max_results, timeout_secs),
        }
    }
}

#[async_trait]
impl Tool for ExaFindSimilarTool {
    fn name(&self) -> &str {
        "exa_find_similar"
    }

    fn description(&self) -> &str {
        "Find web pages semantically similar to a given URL using Exa. Useful for expanding \
         from one good source to comparable ones (competitors, related papers, similar articles)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to find similar pages for."
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum number of results to return (default from config, max 20)."
                },
                "exclude_source_domain": {
                    "type": "boolean",
                    "description": "Exclude other pages from the source URL's own domain."
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
                },
                "include_text": {
                    "type": "boolean",
                    "description": "Also crawl and return the page text for each result (slower)."
                },
                "include_highlights": {
                    "type": "boolean",
                    "description": "Also return relevant highlight snippets for each result."
                }
            },
            "required": ["url"]
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

        let url = non_empty(args.get("url").and_then(Value::as_str))
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: url"))?;

        let limit = self.client.requested_results(&args);
        let mut body = json!({
            "url": url,
            "numResults": limit,
        });
        if let Some(exclude_source) = args.get("exclude_source_domain").and_then(Value::as_bool) {
            body["excludeSourceDomain"] = json!(exclude_source);
        }
        copy_domain_filter(&args, "include_domains", "includeDomains", &mut body);
        copy_domain_filter(&args, "exclude_domains", "excludeDomains", &mut body);
        if let Some(contents) = contents_request(&args) {
            body["contents"] = contents;
        }

        let results = self.client.post_documents("findSimilar", body).await?;
        Ok(self.client.to_result(
            &results,
            &format!("pages similar to {url}"),
            limit,
            &options,
        ))
    }
}

/// Retrieve full page contents for a list of URLs (`POST /contents`).
pub struct ExaGetContentsTool {
    client: ExaClient,
}

impl ExaGetContentsTool {
    pub fn new(
        api_key: Option<String>,
        api_url: Option<String>,
        max_results: usize,
        timeout_secs: u64,
    ) -> Self {
        Self {
            client: ExaClient::new(api_key, api_url, max_results, timeout_secs),
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
        Ok(urls)
    }
}

#[async_trait]
impl Tool for ExaGetContentsTool {
    fn name(&self) -> &str {
        "exa_get_contents"
    }

    fn description(&self) -> &str {
        "Retrieve the full crawled contents of one or more URLs using Exa. Returns page text \
         and, on request, a summary or query-relevant highlights per URL."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "urls": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "The page URLs to retrieve contents for."
                },
                "include_summary": {
                    "type": "boolean",
                    "description": "Also return an Exa-generated summary of each page."
                },
                "include_highlights": {
                    "type": "boolean",
                    "description": "Also return relevant highlight snippets for each page."
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

        // `urls` is Exa's current field name for /contents; `ids` is the
        // backwards-compatible legacy alias, and the schema rejects both at
        // once. Do not "fix" this to `ids`.
        let mut body = json!({
            "urls": urls,
            "text": true,
        });
        if args
            .get("include_summary")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            body["summary"] = json!(true);
        }
        if args
            .get("include_highlights")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            body["highlights"] = json!(true);
        }

        let results = self.client.post_documents("contents", body).await?;
        Ok(self.client.to_result(
            &results,
            &format!("{} URL(s)", urls.len()),
            urls.len(),
            &options,
        ))
    }
}

#[cfg(test)]
#[path = "exa_tests.rs"]
mod tests;
