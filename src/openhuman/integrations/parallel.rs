//! Parallel web search and content extraction integration tools.
//!
//! **Scope**: All (agent loop + CLI/RPC).
//!
//! **Endpoints**:
//!   - `POST /agent-integrations/parallel/search`
//!   - `POST /agent-integrations/parallel/extract`
//!   - `POST /agent-integrations/parallel/chat`
//!   - `POST /agent-integrations/parallel/research` (async; we always wait inline)
//!   - `POST /agent-integrations/parallel/enrich`
//!   - `POST /agent-integrations/parallel/dataset`  (FindAll, async)
//!
//! **Pricing** (fetched from backend):
//!   - Search:  ~$0.01/request
//!   - Extract: ~$0.002/URL
//!   - Chat / research / enrich: per-model or per-processor (see backend `/pricing`)
//!   - Dataset: pre-charged at `match_limit × per-match`
//!
//! The backend handles Parallel API keys, billing, and rate limiting.

use super::IntegrationClient;
use crate::openhuman::tools::traits::{Tool, ToolResult};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

/// UTF-8 safe truncation: returns the truncated slice and whether it was truncated.
fn truncate_chars(s: &str, max_chars: usize) -> (&str, bool) {
    match s.char_indices().nth(max_chars) {
        Some((byte_idx, _)) => (&s[..byte_idx], true),
        None => (s, false),
    }
}

// ── Response types ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(crate) struct SearchResponse {
    #[serde(rename = "searchId")]
    #[allow(dead_code)]
    pub(crate) search_id: String,
    pub(crate) results: Vec<SearchResultItem>,
    #[serde(rename = "costUsd")]
    pub(crate) cost_usd: f64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SearchResultItem {
    pub(crate) url: String,
    pub(crate) title: String,
    pub(crate) publish_date: Option<String>,
    pub(crate) excerpts: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ExtractResponse {
    #[serde(rename = "extractId", default)]
    #[allow(dead_code)]
    extract_id: String,
    #[serde(default)]
    results: Vec<ExtractResultItem>,
    #[serde(default)]
    errors: Vec<ExtractError>,
    #[serde(rename = "costUsd", default)]
    cost_usd: f64,
}

#[derive(Debug, Deserialize)]
struct ExtractResultItem {
    #[serde(default)]
    url: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    excerpts: Vec<String>,
    #[serde(default)]
    full_content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ExtractError {
    #[serde(default)]
    url: String,
    #[serde(default)]
    error: String,
}

// ── ParallelSearchTool ──────────────────────────────────────────────

/// AI-powered web search via the Parallel API.
pub struct ParallelSearchTool {
    client: Arc<IntegrationClient>,
}

impl ParallelSearchTool {
    pub fn new(client: Arc<IntegrationClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for ParallelSearchTool {
    fn name(&self) -> &str {
        "parallel_search"
    }

    fn description(&self) -> &str {
        "AI-powered web search via Parallel. Provide an objective and one or more search \
         queries. Returns relevant results with titles, URLs, and excerpts. \
         Supports modes: 'fast' (quickest), 'one-shot' (balanced), 'agentic' (most thorough). \
         Cost is per request, billed by the backend."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "objective": {
                    "type": "string",
                    "description": "What you are trying to find or learn"
                },
                "search_queries": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "One or more search queries (1-10)",
                    "minItems": 1,
                    "maxItems": 10
                },
                "mode": {
                    "type": "string",
                    "enum": ["fast", "one-shot", "agentic"],
                    "description": "Search mode (default: fast)",
                    "default": "fast"
                },
                "num_results": {
                    "type": "integer",
                    "description": "Number of results per query (1-50, default 10)",
                    "minimum": 1,
                    "maximum": 50,
                    "default": 10
                },
                "max_characters_per_excerpt": {
                    "type": "integer",
                    "description": "Max characters per excerpt (100-10000, default 500)",
                    "minimum": 100,
                    "maximum": 10000,
                    "default": 500
                }
            },
            "required": ["objective", "search_queries"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let objective = args
            .get("objective")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: objective"))?;

        if objective.trim().is_empty() {
            return Ok(ToolResult::error("objective cannot be empty"));
        }

        let search_queries = args
            .get("search_queries")
            .and_then(|v| v.as_array())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: search_queries"))?;

        if search_queries.is_empty() {
            return Ok(ToolResult::error(
                "search_queries must contain at least one query",
            ));
        }

        let mut queries: Vec<&str> = Vec::with_capacity(search_queries.len());
        for (i, v) in search_queries.iter().enumerate() {
            match v.as_str() {
                Some(s) if !s.trim().is_empty() => queries.push(s),
                Some(_) => {
                    return Ok(ToolResult::error(format!(
                        "search_queries[{i}] is an empty string"
                    )));
                }
                None => {
                    return Ok(ToolResult::error(format!(
                        "search_queries[{i}] is not a string"
                    )));
                }
            }
        }

        let mode = args.get("mode").and_then(|v| v.as_str()).unwrap_or("fast");

        let mut body = json!({
            "objective": objective,
            "searchQueries": queries,
            "mode": mode,
        });

        // Build excerpts config if custom values provided
        let num_results = args.get("num_results").and_then(|v| v.as_u64());
        let max_chars = args
            .get("max_characters_per_excerpt")
            .and_then(|v| v.as_u64());

        if num_results.is_some() || max_chars.is_some() {
            let mut excerpts = json!({});
            if let Some(n) = num_results {
                excerpts["numResults"] = json!(n.clamp(1, 50));
            }
            if let Some(c) = max_chars {
                excerpts["maxCharactersPerExcerpt"] = json!(c.clamp(100, 10000));
            }
            body["excerpts"] = excerpts;
        }

        tracing::info!("[parallel_search] queries={}", queries.len());
        tracing::debug!("[parallel_search] objective={:?}", objective);

        match self
            .client
            .post::<SearchResponse>("/agent-integrations/parallel/search", &body)
            .await
        {
            Ok(resp) => {
                if resp.results.is_empty() {
                    return Ok(ToolResult::success(format!(
                        "No results found for: {}",
                        objective
                    )));
                }

                let mut lines = vec![format!("Search results ({} found):", resp.results.len())];

                for (i, item) in resp.results.iter().enumerate() {
                    lines.push(format!("\n{}. {}", i + 1, item.title));
                    lines.push(format!("   {}", item.url));
                    if let Some(ref date) = item.publish_date {
                        lines.push(format!("   Published: {}", date));
                    }
                    if let Some(excerpt) = item.excerpts.first() {
                        let text = excerpt.trim();
                        if !text.is_empty() {
                            let (slice, was_truncated) = truncate_chars(text, 500);
                            let truncated = if was_truncated {
                                format!("{slice}...")
                            } else {
                                slice.to_string()
                            };
                            lines.push(format!("   {}", truncated));
                        }
                    }
                }

                lines.push(format!("\nCost: ${:.4}", resp.cost_usd));
                Ok(ToolResult::success(lines.join("\n")))
            }
            Err(e) => Ok(ToolResult::error(format!("Parallel search failed: {e}"))),
        }
    }
}

// ── ParallelExtractTool ─────────────────────────────────────────────

/// Extract content from web pages via the Parallel API.
pub struct ParallelExtractTool {
    client: Arc<IntegrationClient>,
}

impl ParallelExtractTool {
    pub fn new(client: Arc<IntegrationClient>) -> Self {
        Self { client }
    }
}

/// Maximum characters of full_content to include per URL in tool output.
const MAX_CONTENT_CHARS: usize = 5000;

#[async_trait]
impl Tool for ParallelExtractTool {
    fn name(&self) -> &str {
        "parallel_extract"
    }

    fn description(&self) -> &str {
        "Extract content from one or more web pages using the Parallel API. \
         Returns page titles, excerpts, and optionally full content. \
         Useful for reading articles, documentation, or structured data from URLs. \
         Cost is per URL, billed by the backend."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "urls": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "URLs to extract content from (1-20)",
                    "minItems": 1,
                    "maxItems": 20
                },
                "objective": {
                    "type": "string",
                    "description": "What information to focus on when extracting"
                },
                "excerpts": {
                    "type": "boolean",
                    "description": "Include relevant excerpts (default true)",
                    "default": true
                },
                "full_content": {
                    "type": "boolean",
                    "description": "Include full page content (default false)",
                    "default": false
                }
            },
            "required": ["urls"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let urls = args
            .get("urls")
            .and_then(|v| v.as_array())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: urls"))?;

        if urls.is_empty() {
            return Ok(ToolResult::error("urls must contain at least one URL"));
        }

        let mut url_strings: Vec<&str> = Vec::with_capacity(urls.len());
        for (i, v) in urls.iter().enumerate() {
            match v.as_str() {
                Some(s) if !s.trim().is_empty() => url_strings.push(s),
                Some(_) => {
                    return Ok(ToolResult::error(format!("urls[{i}] is an empty string")));
                }
                None => {
                    return Ok(ToolResult::error(format!("urls[{i}] is not a string")));
                }
            }
        }

        let objective = args.get("objective").and_then(|v| v.as_str());
        let excerpts = args
            .get("excerpts")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let full_content = args
            .get("full_content")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let mut body = json!({
            "urls": url_strings,
            "excerpts": excerpts,
            "fullContent": full_content,
        });

        if let Some(obj) = objective {
            body["objective"] = json!(obj);
        }

        tracing::info!("[parallel_extract] urls={}", url_strings.len());

        match self
            .client
            .post::<ExtractResponse>("/agent-integrations/parallel/extract", &body)
            .await
        {
            Ok(resp) => {
                let mut lines = Vec::new();

                for (i, item) in resp.results.iter().enumerate() {
                    let title = item.title.as_deref().unwrap_or("(no title)");
                    lines.push(format!("\n{}. {} — {}", i + 1, title, item.url));

                    for excerpt in &item.excerpts {
                        let text = excerpt.trim();
                        if !text.is_empty() {
                            let (slice, was_truncated) = truncate_chars(text, 500);
                            let truncated = if was_truncated {
                                format!("{slice}...")
                            } else {
                                slice.to_string()
                            };
                            lines.push(format!("   {}", truncated));
                        }
                    }

                    if let Some(ref content) = item.full_content {
                        let content = content.trim();
                        if !content.is_empty() {
                            let (slice, was_truncated) = truncate_chars(content, MAX_CONTENT_CHARS);
                            let truncated = if was_truncated {
                                format!(
                                    "{}... [truncated, {} chars total]",
                                    slice,
                                    content.chars().count()
                                )
                            } else {
                                slice.to_string()
                            };
                            lines.push(format!("   Content:\n   {}", truncated));
                        }
                    }
                }

                if !resp.errors.is_empty() {
                    lines.push("\nErrors:".to_string());
                    for err in &resp.errors {
                        lines.push(format!("  {} — {}", err.url, err.error));
                    }
                }

                if lines.is_empty() {
                    Ok(ToolResult::success(format!(
                        "No content extracted from the provided URLs.\nCost: ${:.4}",
                        resp.cost_usd
                    )))
                } else {
                    lines.push(format!("\nCost: ${:.4}", resp.cost_usd));
                    Ok(ToolResult::success(lines.join("\n")))
                }
            }
            Err(e) => Ok(ToolResult::error(format!("Parallel extract failed: {e}"))),
        }
    }
}

// ── ParallelChatTool ────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct ChatResponse {
    #[serde(default)]
    choices: Vec<ChatChoice>,
    #[serde(default)]
    basis: Option<serde_json::Value>,
    #[serde(rename = "costUsd", default)]
    cost_usd: f64,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    #[serde(default)]
    message: ChatMessage,
    #[serde(default, rename = "finish_reason")]
    finish_reason: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct ChatMessage {
    #[serde(default)]
    #[allow(dead_code)]
    role: String,
    #[serde(default)]
    content: String,
}

/// AI-powered chat backed by Parallel's web-research models.
pub struct ParallelChatTool {
    client: Arc<IntegrationClient>,
}

impl ParallelChatTool {
    pub fn new(client: Arc<IntegrationClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for ParallelChatTool {
    fn name(&self) -> &str {
        "parallel_chat"
    }

    fn description(&self) -> &str {
        "Chat with the web via Parallel's research-grounded chat models. \
         OpenAI-compatible: pass `messages` (system/user/assistant) and a `model` \
         (`speed` cheapest, `lite`, `base`, `core` most capable). Returns the \
         assistant's reply plus optional citation `basis` for research models. \
         Use this for grounded Q&A like \"summarize today's news on …\" or \
         \"what is the current price of BTC?\" — Parallel will browse and cite."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "model": {
                    "type": "string",
                    "enum": ["speed", "lite", "base", "core"],
                    "description": "Model tier — speed (cheapest) → core (most capable)"
                },
                "messages": {
                    "type": "array",
                    "minItems": 1,
                    "items": {
                        "type": "object",
                        "properties": {
                            "role": { "type": "string", "enum": ["system", "user", "assistant"] },
                            "content": { "type": "string" }
                        },
                        "required": ["role", "content"]
                    }
                }
            },
            "required": ["model", "messages"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let model = args
            .get("model")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: model"))?;
        let messages = args
            .get("messages")
            .and_then(|v| v.as_array())
            .filter(|a| !a.is_empty())
            .ok_or_else(|| anyhow::anyhow!("messages must be a non-empty array"))?;

        tracing::info!(
            "[parallel_chat] model={} messages={}",
            model,
            messages.len()
        );

        let body = json!({ "model": model, "messages": messages });
        match self
            .client
            .post::<ChatResponse>("/agent-integrations/parallel/chat", &body)
            .await
        {
            Ok(resp) => {
                let mut out = String::new();
                if let Some(c) = resp.choices.first() {
                    out.push_str(&c.message.content);
                    if let Some(reason) = &c.finish_reason {
                        out.push_str(&format!("\n\n[finish_reason: {}]", reason));
                    }
                } else {
                    out.push_str("(no choices returned)");
                }
                if let Some(basis) = resp.basis {
                    out.push_str(&format!(
                        "\n\nCitations (basis):\n{}",
                        serde_json::to_string_pretty(&basis).unwrap_or_default()
                    ));
                }
                out.push_str(&format!("\n\nCost: ${:.4}", resp.cost_usd));
                Ok(ToolResult::success(out))
            }
            Err(e) => Ok(ToolResult::error(format!("Parallel chat failed: {e}"))),
        }
    }
}

// ── ParallelResearchTool ────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct ResearchResponse {
    #[serde(default, rename = "runId")]
    run_id: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    result: Option<serde_json::Value>,
    #[serde(rename = "costUsd", default)]
    cost_usd: f64,
}

/// Deep research via Parallel's Task API — multi-step web investigation
/// with structured or freeform output.
pub struct ParallelResearchTool {
    client: Arc<IntegrationClient>,
}

impl ParallelResearchTool {
    pub fn new(client: Arc<IntegrationClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for ParallelResearchTool {
    fn name(&self) -> &str {
        "parallel_research"
    }

    fn description(&self) -> &str {
        "Deep web research via Parallel's Task API. Submit an objective and a processor \
         tier (`lite`, `base`, `core`, `ultra`) — Parallel browses many sources, \
         synthesises, and returns a single rich answer. Optionally pass an \
         `output_schema` (JSON schema) to force structured output. \
         Blocks inline until the run completes (up to ~10 minutes). \
         Use for tasks that need more than a single search/extract pair, e.g. \
         \"compare these three companies' financials\" or \"build a competitor matrix\"."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "input": {
                    "description": "The research objective — string or structured object",
                    "oneOf": [{ "type": "string" }, { "type": "object" }]
                },
                "processor": {
                    "type": "string",
                    "enum": ["lite", "base", "core", "ultra"],
                    "description": "Processor tier — lite (cheapest) → ultra (most thorough)"
                },
                "output_schema": {
                    "type": "object",
                    "description": "Optional JSON schema describing the desired structured output"
                },
                "timeout_seconds": {
                    "type": "integer",
                    "minimum": 10,
                    "maximum": 900,
                    "description": "Max time to wait inline (default 600)"
                }
            },
            "required": ["input", "processor"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let input = args
            .get("input")
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: input"))?;
        let processor = args
            .get("processor")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: processor"))?;

        let mut body = json!({
            "input": input,
            "processor": processor,
            "wait": true,
        });
        if let Some(schema) = args.get("output_schema") {
            body["outputSchema"] = schema.clone();
        }
        if let Some(t) = args.get("timeout_seconds").and_then(|v| v.as_u64()) {
            body["timeoutSeconds"] = json!(t.clamp(10, 900));
        }

        tracing::info!("[parallel_research] processor={}", processor);

        match self
            .client
            .post::<ResearchResponse>("/agent-integrations/parallel/research", &body)
            .await
        {
            Ok(resp) => {
                let mut out = String::new();
                if let Some(id) = &resp.run_id {
                    out.push_str(&format!("Run: {}\n", id));
                }
                if let Some(s) = &resp.status {
                    out.push_str(&format!("Status: {}\n", s));
                }
                if let Some(r) = resp.result {
                    out.push_str("\nResult:\n");
                    out.push_str(&serde_json::to_string_pretty(&r).unwrap_or_default());
                } else {
                    out.push_str("\n(no result returned — run may still be in progress)");
                }
                out.push_str(&format!("\n\nCost: ${:.4}", resp.cost_usd));
                Ok(ToolResult::success(out))
            }
            Err(e) => Ok(ToolResult::error(format!("Parallel research failed: {e}"))),
        }
    }
}

// ── ParallelEnrichTool ──────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct EnrichResponse {
    #[serde(default, rename = "runId")]
    run_id: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    output: Option<serde_json::Value>,
    #[serde(rename = "costUsd", default)]
    cost_usd: f64,
}

/// Enrich an entity with structured web data — synchronous Task API run
/// with a required output schema.
pub struct ParallelEnrichTool {
    client: Arc<IntegrationClient>,
}

impl ParallelEnrichTool {
    pub fn new(client: Arc<IntegrationClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for ParallelEnrichTool {
    fn name(&self) -> &str {
        "parallel_enrich"
    }

    fn description(&self) -> &str {
        "Enrich an entity (company, person, product) with structured web data. \
         Pass an `input` (the thing to enrich) plus a JSON `output_schema` describing \
         the fields you want filled in — Parallel returns a structured object \
         conforming to that schema. Blocks until the run completes."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "input": {
                    "description": "Entity to enrich — string or object",
                    "oneOf": [{ "type": "string" }, { "type": "object" }]
                },
                "processor": {
                    "type": "string",
                    "enum": ["lite", "base", "core", "ultra"],
                    "description": "Processor tier — lite (cheapest) → ultra (most thorough)"
                },
                "output_schema": {
                    "type": "object",
                    "description": "JSON schema for the structured output (required)"
                },
                "timeout_seconds": {
                    "type": "integer",
                    "minimum": 10,
                    "maximum": 900,
                    "description": "Max time to wait (default 600)"
                }
            },
            "required": ["input", "processor", "output_schema"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let input = args
            .get("input")
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: input"))?;
        let processor = args
            .get("processor")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: processor"))?;
        let output_schema = args
            .get("output_schema")
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: output_schema"))?;

        let mut body = json!({
            "input": input,
            "processor": processor,
            "outputSchema": output_schema,
        });
        if let Some(t) = args.get("timeout_seconds").and_then(|v| v.as_u64()) {
            body["timeoutSeconds"] = json!(t.clamp(10, 900));
        }

        tracing::info!("[parallel_enrich] processor={}", processor);

        match self
            .client
            .post::<EnrichResponse>("/agent-integrations/parallel/enrich", &body)
            .await
        {
            Ok(resp) => {
                let mut out = String::new();
                if let Some(id) = &resp.run_id {
                    out.push_str(&format!("Run: {}\n", id));
                }
                if let Some(s) = &resp.status {
                    out.push_str(&format!("Status: {}\n", s));
                }
                if let Some(o) = resp.output {
                    out.push_str("\nOutput:\n");
                    out.push_str(&serde_json::to_string_pretty(&o).unwrap_or_default());
                }
                out.push_str(&format!("\n\nCost: ${:.4}", resp.cost_usd));
                Ok(ToolResult::success(out))
            }
            Err(e) => Ok(ToolResult::error(format!("Parallel enrich failed: {e}"))),
        }
    }
}

// ── ParallelDatasetTool ─────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct DatasetResponse {
    #[serde(rename = "findallId", default)]
    findall_id: String,
    #[serde(default)]
    status: serde_json::Value,
    #[serde(rename = "matchLimit", default)]
    match_limit: u64,
    #[serde(rename = "costUsd", default)]
    cost_usd: f64,
}

/// Generate a web dataset via Parallel's FindAll — kicks off an async run
/// that produces structured candidate matches.
pub struct ParallelDatasetTool {
    client: Arc<IntegrationClient>,
}

impl ParallelDatasetTool {
    pub fn new(client: Arc<IntegrationClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for ParallelDatasetTool {
    fn name(&self) -> &str {
        "parallel_dataset"
    }

    fn description(&self) -> &str {
        "Generate a web dataset via Parallel FindAll. Describe an `objective`, \
         the `entity_type` you want (e.g. \"SaaS company\", \"academic paper\"), \
         and a list of `match_conditions` — each a `name` plus an optional \
         `description`. Parallel discovers and enriches matching candidates \
         in the background. This call returns the run ID and pre-authorised cost; \
         use `match_limit` to cap how many candidates are produced. \
         Run is async — fetch results separately by `findall_id`."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "objective": { "type": "string", "description": "What dataset to build" },
                "entity_type": { "type": "string", "description": "What kind of entity to find" },
                "match_conditions": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": 20,
                    "items": {
                        "type": "object",
                        "properties": {
                            "name": { "type": "string" },
                            "description": { "type": "string" }
                        },
                        "required": ["name"]
                    }
                },
                "generator": {
                    "type": "string",
                    "enum": ["preview", "base", "core", "pro"],
                    "description": "Generator tier (default base)"
                },
                "match_limit": {
                    "type": "integer",
                    "minimum": 5,
                    "maximum": 1000,
                    "description": "Max candidates to produce (default 10)"
                }
            },
            "required": ["objective", "entity_type", "match_conditions"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let objective = args
            .get("objective")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: objective"))?;
        let entity_type = args
            .get("entity_type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: entity_type"))?;
        let match_conditions = args
            .get("match_conditions")
            .and_then(|v| v.as_array())
            .filter(|a| !a.is_empty())
            .ok_or_else(|| anyhow::anyhow!("match_conditions must be a non-empty array"))?;

        let mut body = json!({
            "objective": objective,
            "entityType": entity_type,
            "matchConditions": match_conditions,
        });
        if let Some(g) = args.get("generator").and_then(|v| v.as_str()) {
            body["generator"] = json!(g);
        }
        if let Some(l) = args.get("match_limit").and_then(|v| v.as_u64()) {
            body["matchLimit"] = json!(l.clamp(5, 1000));
        }

        tracing::info!("[parallel_dataset] entity_type={}", entity_type);

        match self
            .client
            .post::<DatasetResponse>("/agent-integrations/parallel/dataset", &body)
            .await
        {
            Ok(resp) => {
                let out = format!(
                    "Dataset run started\n  findall_id: {}\n  match_limit: {}\n  status: {}\n\nCost (pre-authorised): ${:.4}\n\nResults are produced asynchronously — fetch them later by findall_id.",
                    resp.findall_id,
                    resp.match_limit,
                    serde_json::to_string(&resp.status).unwrap_or_default(),
                    resp.cost_usd
                );
                Ok(ToolResult::success(out))
            }
            Err(e) => Ok(ToolResult::error(format!("Parallel dataset failed: {e}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::integrations::ToolScope;

    fn test_client() -> Arc<IntegrationClient> {
        Arc::new(IntegrationClient::new("http://test".into(), "tok".into()))
    }

    // ── ParallelSearchTool ──────────────────────────────────────────

    #[test]
    fn search_tool_metadata() {
        let tool = ParallelSearchTool::new(test_client());
        assert_eq!(tool.name(), "parallel_search");
        assert_eq!(tool.scope(), ToolScope::All);
        assert!(tool.description().contains("web search"));
    }

    #[test]
    fn search_schema_required_fields() {
        let tool = ParallelSearchTool::new(test_client());
        let schema = tool.parameters_schema();
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "objective"));
        assert!(required.iter().any(|v| v == "search_queries"));
    }

    #[tokio::test]
    async fn search_rejects_missing_objective() {
        let tool = ParallelSearchTool::new(test_client());
        assert!(tool
            .execute(json!({"search_queries": ["test"]}))
            .await
            .is_err());
    }

    #[tokio::test]
    async fn search_rejects_empty_objective() {
        let tool = ParallelSearchTool::new(test_client());
        let result = tool
            .execute(json!({"objective": "", "search_queries": ["test"]}))
            .await
            .unwrap();
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn search_rejects_empty_queries() {
        let tool = ParallelSearchTool::new(test_client());
        let result = tool
            .execute(json!({"objective": "test", "search_queries": []}))
            .await
            .unwrap();
        assert!(result.is_error);
    }

    #[test]
    fn search_response_rejects_missing_search_id() {
        let json = r#"{
            "results": [],
            "costUsd": 0.01
        }"#;
        assert!(serde_json::from_str::<SearchResponse>(json).is_err());
    }

    #[test]
    fn search_response_rejects_missing_results() {
        let json = r#"{
            "searchId": "s123",
            "costUsd": 0.01
        }"#;
        assert!(serde_json::from_str::<SearchResponse>(json).is_err());
    }

    #[test]
    fn search_response_rejects_missing_cost_usd() {
        let json = r#"{
            "searchId": "s123",
            "results": []
        }"#;
        assert!(serde_json::from_str::<SearchResponse>(json).is_err());
    }

    #[test]
    fn search_response_deserializes() {
        let json = r#"{
            "searchId": "s123",
            "results": [
                {
                    "url": "https://example.com",
                    "title": "Example",
                    "publish_date": "2026-01-01",
                    "excerpts": ["Some text"]
                }
            ],
            "costUsd": 0.01
        }"#;
        let resp: SearchResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.results.len(), 1);
        assert_eq!(resp.results[0].title, "Example");
    }

    // ── ParallelExtractTool ─────────────────────────────────────────

    #[test]
    fn extract_tool_metadata() {
        let tool = ParallelExtractTool::new(test_client());
        assert_eq!(tool.name(), "parallel_extract");
        assert_eq!(tool.scope(), ToolScope::All);
        assert!(tool.description().contains("Extract content"));
    }

    #[test]
    fn extract_schema_required_urls() {
        let tool = ParallelExtractTool::new(test_client());
        let schema = tool.parameters_schema();
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "urls"));
    }

    #[tokio::test]
    async fn extract_rejects_missing_urls() {
        let tool = ParallelExtractTool::new(test_client());
        assert!(tool.execute(json!({})).await.is_err());
    }

    #[tokio::test]
    async fn extract_rejects_empty_urls() {
        let tool = ParallelExtractTool::new(test_client());
        let result = tool.execute(json!({"urls": []})).await.unwrap();
        assert!(result.is_error);
    }

    #[test]
    fn extract_response_deserializes() {
        let json = r#"{
            "extractId": "e123",
            "results": [
                {
                    "url": "https://example.com",
                    "title": "Example Page",
                    "excerpts": ["Key info here"],
                    "full_content": null
                }
            ],
            "errors": [
                {"url": "https://bad.com", "error": "timeout"}
            ],
            "costUsd": 0.002
        }"#;
        let resp: ExtractResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.results.len(), 1);
        assert_eq!(resp.errors.len(), 1);
        assert_eq!(resp.errors[0].url, "https://bad.com");
    }

    #[test]
    fn extract_response_with_full_content() {
        let json = r#"{
            "extractId": "e456",
            "results": [
                {
                    "url": "https://example.com",
                    "title": "Full Article",
                    "excerpts": [],
                    "full_content": "This is the full article content."
                }
            ],
            "errors": [],
            "costUsd": 0.002
        }"#;
        let resp: ExtractResponse = serde_json::from_str(json).unwrap();
        assert_eq!(
            resp.results[0].full_content.as_deref(),
            Some("This is the full article content.")
        );
    }

    // ── ParallelChatTool / ResearchTool / EnrichTool / DatasetTool ──

    #[test]
    fn chat_tool_metadata() {
        let t = ParallelChatTool::new(test_client());
        assert_eq!(t.name(), "parallel_chat");
        assert_eq!(t.scope(), ToolScope::All);
        let req = t.parameters_schema()["required"]
            .as_array()
            .unwrap()
            .clone();
        assert!(req.iter().any(|v| v == "model"));
        assert!(req.iter().any(|v| v == "messages"));
    }

    #[test]
    fn research_tool_metadata() {
        let t = ParallelResearchTool::new(test_client());
        assert_eq!(t.name(), "parallel_research");
    }

    #[test]
    fn enrich_tool_metadata() {
        let t = ParallelEnrichTool::new(test_client());
        assert_eq!(t.name(), "parallel_enrich");
        let req = t.parameters_schema()["required"]
            .as_array()
            .unwrap()
            .clone();
        assert!(req.iter().any(|v| v == "output_schema"));
    }

    #[test]
    fn dataset_tool_metadata() {
        let t = ParallelDatasetTool::new(test_client());
        assert_eq!(t.name(), "parallel_dataset");
    }

    #[tokio::test]
    async fn chat_rejects_missing_messages() {
        let t = ParallelChatTool::new(test_client());
        assert!(t.execute(json!({"model": "speed"})).await.is_err());
    }

    #[tokio::test]
    async fn research_rejects_missing_processor() {
        let t = ParallelResearchTool::new(test_client());
        assert!(t.execute(json!({"input": "x"})).await.is_err());
    }

    #[tokio::test]
    async fn enrich_rejects_missing_output_schema() {
        let t = ParallelEnrichTool::new(test_client());
        assert!(t
            .execute(json!({"input": "x", "processor": "lite"}))
            .await
            .is_err());
    }

    #[tokio::test]
    async fn dataset_rejects_empty_match_conditions() {
        let t = ParallelDatasetTool::new(test_client());
        assert!(t
            .execute(json!({
                "objective": "x",
                "entity_type": "y",
                "match_conditions": []
            }))
            .await
            .is_err());
    }

    #[test]
    fn chat_response_deserializes() {
        let json = r#"{
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": "BTC is $77k" },
                "finish_reason": "stop"
            }],
            "costUsd": 0.005
        }"#;
        let resp: ChatResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.choices.len(), 1);
        assert_eq!(resp.choices[0].message.content, "BTC is $77k");
    }

    #[test]
    fn dataset_response_deserializes() {
        let json = r#"{
            "findallId": "fa_123",
            "status": { "status": "queued", "is_active": true },
            "matchLimit": 25,
            "costUsd": 0.05
        }"#;
        let resp: DatasetResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.findall_id, "fa_123");
        assert_eq!(resp.match_limit, 25);
    }
}
