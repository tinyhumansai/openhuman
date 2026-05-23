//! Brave Search direct-API integration.
//!
//! **Scope**: Agent + CLI/RPC.
//!
//! Brave exposes several REST endpoints — we layer one tool per
//! endpoint so the agent can pick the right surface:
//!
//! - `GET https://api.search.brave.com/res/v1/web/search`    → `brave_web_search`
//! - `GET https://api.search.brave.com/res/v1/news/search`   → `brave_news_search`
//! - `GET https://api.search.brave.com/res/v1/images/search` → `brave_image_search`
//! - `GET https://api.search.brave.com/res/v1/videos/search` → `brave_video_search`
//!
//! **Auth**: `X-Subscription-Token: <api_key>` header.
//!
//! Each tool short-circuits with a clear "not configured" error when no
//! key is wired up so the agent surfaces the misconfiguration instead
//! of silently routing elsewhere.

use crate::openhuman::tools::traits::{Tool, ToolCallOptions, ToolResult};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;

const DEFAULT_API_BASE: &str = "https://api.search.brave.com/res/v1";

fn http_client(timeout_secs: u64) -> anyhow::Result<reqwest::Client> {
    crate::openhuman::tls::tls_client_builder()
        .timeout(Duration::from_secs(timeout_secs.max(1)))
        .connect_timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| anyhow::anyhow!("failed to build Brave HTTP client: {e}"))
}

#[derive(Debug, Clone)]
struct BraveConfig {
    api_key: Option<String>,
    max_results: usize,
    timeout_secs: u64,
}

impl BraveConfig {
    fn require_key(&self) -> anyhow::Result<&str> {
        self.api_key
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Brave Search unavailable: no API key configured. \
                     Set OPENHUMAN_BRAVE_API_KEY or `search.brave.api_key` \
                     in config.toml and select `search.engine = \"brave\"`."
                )
            })
    }
}

async fn brave_get(
    cfg: &BraveConfig,
    path: &str,
    query: &[(&str, String)],
) -> anyhow::Result<Value> {
    let key = cfg.require_key()?;
    let url = format!("{DEFAULT_API_BASE}{path}");
    tracing::debug!(
        path = %path,
        timeout_secs = cfg.timeout_secs,
        key_present = true,
        param_count = query.len(),
        "[brave] GET request"
    );
    let client = http_client(cfg.timeout_secs)?;
    let started = std::time::Instant::now();
    let resp = client
        .get(&url)
        .header("X-Subscription-Token", key)
        .header("Accept", "application/json")
        .header("Accept-Encoding", "gzip")
        .query(query)
        .send()
        .await
        .map_err(|e| {
            tracing::warn!(path = %path, "[brave] request send failed: {e}");
            anyhow::anyhow!("Brave request failed: {e}")
        })?;
    let status = resp.status();
    let elapsed_ms = started.elapsed().as_millis();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        let detail = crate::openhuman::util::utf8_safe_prefix_at_byte_boundary(&body, 500);
        tracing::warn!(
            path = %path,
            status = %status,
            elapsed_ms = elapsed_ms as u64,
            "[brave] non-2xx response: {detail}"
        );
        anyhow::bail!("Brave returned {status}: {detail}");
    }
    tracing::debug!(
        path = %path,
        status = %status,
        elapsed_ms = elapsed_ms as u64,
        body_bytes = body.len(),
        "[brave] response ok"
    );
    serde_json::from_str::<Value>(&body).map_err(|e| {
        tracing::warn!(path = %path, "[brave] JSON parse failed: {e}");
        anyhow::anyhow!("Brave returned malformed JSON: {e}")
    })
}

fn extract_query(args: &Value) -> anyhow::Result<String> {
    let q = args
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing required parameter: query"))?
        .trim();
    if q.is_empty() {
        anyhow::bail!("Search query cannot be empty");
    }
    Ok(q.to_string())
}

fn clamped_count(args: &Value, default: usize, max: usize) -> usize {
    args.get("count")
        .and_then(|v| v.as_u64())
        .map(|n| (n as usize).clamp(1, max))
        .unwrap_or_else(|| default.clamp(1, max))
}

fn pub_string(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

// ── Web ─────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct WebResp {
    web: Option<WebContainer>,
}
#[derive(Debug, Deserialize)]
struct WebContainer {
    #[serde(default)]
    results: Vec<WebResult>,
}
#[derive(Debug, Deserialize)]
struct WebResult {
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    age: Option<String>,
}

pub struct BraveWebSearchTool {
    cfg: BraveConfig,
}

impl BraveWebSearchTool {
    pub fn new(api_key: Option<String>, max_results: usize, timeout_secs: u64) -> Self {
        Self {
            cfg: BraveConfig {
                api_key,
                max_results: max_results.clamp(1, 20),
                timeout_secs: timeout_secs.max(1),
            },
        }
    }
}

#[async_trait]
impl Tool for BraveWebSearchTool {
    fn name(&self) -> &str {
        "web_search_tool"
    }

    fn description(&self) -> &str {
        "Search the web via Brave Search. Returns ranked results with titles, URLs, and descriptions."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Search query (concise keywords work best)." },
                "count": { "type": "integer", "description": "Max results (1-20)." },
                "country": { "type": "string", "description": "2-letter country code (e.g. 'us', 'gb')." },
                "freshness": { "type": "string", "description": "Time filter: 'pd' (past day), 'pw' (past week), 'pm' (past month), 'py' (past year), or 'YYYY-MM-DDtoYYYY-MM-DD'." }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        self.execute_with_options(args, ToolCallOptions::default())
            .await
    }

    fn supports_markdown(&self) -> bool {
        true
    }

    async fn execute_with_options(
        &self,
        args: Value,
        options: ToolCallOptions,
    ) -> anyhow::Result<ToolResult> {
        let query = extract_query(&args)?;
        let count = clamped_count(&args, self.cfg.max_results, 20);
        let mut q: Vec<(&str, String)> = vec![
            ("q", query.clone()),
            ("count", count.to_string()),
            ("result_filter", "web".into()),
        ];
        if let Some(c) = pub_string(&args, "country") {
            q.push(("country", c));
        }
        if let Some(f) = pub_string(&args, "freshness") {
            q.push(("freshness", f));
        }
        let raw = brave_get(&self.cfg, "/web/search", &q).await?;
        let parsed: WebResp = serde_json::from_value(raw)
            .map_err(|e| anyhow::anyhow!("Brave web response shape changed: {e}"))?;
        let results = parsed.web.map(|w| w.results).unwrap_or_default();
        let plain = render_web_plain(&results, &query, count);
        let mut out = ToolResult::success(plain);
        if options.prefer_markdown {
            out.markdown_formatted = Some(render_web_markdown(&results, &query, count));
        }
        Ok(out)
    }
}

fn render_web_plain(results: &[WebResult], query: &str, max: usize) -> String {
    if results.is_empty() {
        return format!("No results found for: {query}");
    }
    let mut lines = vec![format!("Search results for: {query} (via Brave)")];
    for (i, r) in results.iter().take(max).enumerate() {
        let title = if r.title.trim().is_empty() {
            "Untitled"
        } else {
            r.title.trim()
        };
        lines.push(format!("{}. {}", i + 1, title));
        lines.push(format!("   {}", r.url.trim()));
        if let Some(age) = r.age.as_deref() {
            let age = age.trim();
            if !age.is_empty() {
                lines.push(format!("   {age}"));
            }
        }
        let desc = r.description.trim();
        if !desc.is_empty() {
            lines.push(format!(
                "   {}",
                crate::openhuman::util::truncate_with_ellipsis(desc, 500)
            ));
        }
    }
    lines.join("\n")
}

fn render_web_markdown(results: &[WebResult], query: &str, max: usize) -> String {
    if results.is_empty() {
        return format!("_No results for `{query}`._");
    }
    let mut out = format!("# Search results — `{query}` (Brave)\n");
    for r in results.iter().take(max) {
        let title = if r.title.trim().is_empty() {
            "Untitled"
        } else {
            r.title.trim()
        };
        out.push_str(&format!("\n## [{title}]({})\n", r.url.trim()));
        if let Some(age) = r.age.as_deref() {
            let age = age.trim();
            if !age.is_empty() {
                out.push_str(&format!("_{age}_\n\n"));
            }
        }
        let desc = r.description.trim();
        if !desc.is_empty() {
            out.push_str(&format!(
                "> {}\n",
                crate::openhuman::util::truncate_with_suffix(desc, 500, "…")
            ));
        }
    }
    out
}

// ── News ────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct NewsResp {
    #[serde(default)]
    results: Vec<NewsResult>,
}
#[derive(Debug, Deserialize)]
struct NewsResult {
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    age: Option<String>,
    #[serde(default)]
    source: Option<String>,
}

pub struct BraveNewsSearchTool {
    cfg: BraveConfig,
}

impl BraveNewsSearchTool {
    pub fn new(api_key: Option<String>, max_results: usize, timeout_secs: u64) -> Self {
        Self {
            cfg: BraveConfig {
                api_key,
                max_results: max_results.clamp(1, 20),
                timeout_secs: timeout_secs.max(1),
            },
        }
    }
}

#[async_trait]
impl Tool for BraveNewsSearchTool {
    fn name(&self) -> &str {
        "brave_news_search"
    }

    fn description(&self) -> &str {
        "Search recent news articles via Brave Search News. Returns titles, URLs, sources, and excerpts."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "News topic to search for." },
                "count": { "type": "integer", "description": "Max results (1-20)." },
                "country": { "type": "string", "description": "2-letter country code." },
                "freshness": { "type": "string", "description": "Time filter: 'pd', 'pw', 'pm', 'py'." }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let query = extract_query(&args)?;
        let count = clamped_count(&args, self.cfg.max_results, 20);
        let mut q: Vec<(&str, String)> = vec![("q", query.clone()), ("count", count.to_string())];
        if let Some(c) = pub_string(&args, "country") {
            q.push(("country", c));
        }
        if let Some(f) = pub_string(&args, "freshness") {
            q.push(("freshness", f));
        }
        let raw = brave_get(&self.cfg, "/news/search", &q).await?;
        let parsed: NewsResp = serde_json::from_value(raw)
            .map_err(|e| anyhow::anyhow!("Brave news response shape changed: {e}"))?;
        let mut lines = if parsed.results.is_empty() {
            return Ok(ToolResult::success(format!("No news found for: {query}")));
        } else {
            vec![format!("News results for: {query} (via Brave)")]
        };
        for (i, r) in parsed.results.iter().take(count).enumerate() {
            let title = if r.title.trim().is_empty() {
                "Untitled"
            } else {
                r.title.trim()
            };
            lines.push(format!("{}. {}", i + 1, title));
            lines.push(format!("   {}", r.url.trim()));
            if let Some(src) = r.source.as_deref() {
                let src = src.trim();
                if !src.is_empty() {
                    lines.push(format!("   Source: {src}"));
                }
            }
            if let Some(age) = r.age.as_deref() {
                let age = age.trim();
                if !age.is_empty() {
                    lines.push(format!("   {age}"));
                }
            }
            let desc = r.description.trim();
            if !desc.is_empty() {
                lines.push(format!(
                    "   {}",
                    crate::openhuman::util::truncate_with_ellipsis(desc, 500)
                ));
            }
        }
        Ok(ToolResult::success(lines.join("\n")))
    }
}

// ── Images ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct ImageResp {
    #[serde(default)]
    results: Vec<ImageResult>,
}
#[derive(Debug, Deserialize)]
struct ImageResult {
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    thumbnail: Option<ImageThumb>,
    #[serde(default)]
    properties: Option<ImageProps>,
}
#[derive(Debug, Deserialize)]
struct ImageThumb {
    #[serde(default)]
    src: Option<String>,
}
#[derive(Debug, Deserialize)]
struct ImageProps {
    #[serde(default)]
    url: Option<String>,
}

pub struct BraveImageSearchTool {
    cfg: BraveConfig,
}

impl BraveImageSearchTool {
    pub fn new(api_key: Option<String>, max_results: usize, timeout_secs: u64) -> Self {
        Self {
            cfg: BraveConfig {
                api_key,
                max_results: max_results.clamp(1, 20),
                timeout_secs: timeout_secs.max(1),
            },
        }
    }
}

#[async_trait]
impl Tool for BraveImageSearchTool {
    fn name(&self) -> &str {
        "brave_image_search"
    }

    fn description(&self) -> &str {
        "Search for images via Brave Search. Returns image URLs, source pages, and thumbnails."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Image search query." },
                "count": { "type": "integer", "description": "Max results (1-20)." },
                "country": { "type": "string", "description": "2-letter country code." }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let query = extract_query(&args)?;
        let count = clamped_count(&args, self.cfg.max_results, 20);
        let mut q: Vec<(&str, String)> = vec![("q", query.clone()), ("count", count.to_string())];
        if let Some(c) = pub_string(&args, "country") {
            q.push(("country", c));
        }
        let raw = brave_get(&self.cfg, "/images/search", &q).await?;
        let parsed: ImageResp = serde_json::from_value(raw)
            .map_err(|e| anyhow::anyhow!("Brave image response shape changed: {e}"))?;
        if parsed.results.is_empty() {
            return Ok(ToolResult::success(format!("No images found for: {query}")));
        }
        let mut lines = vec![format!("Image results for: {query} (via Brave)")];
        for (i, r) in parsed.results.iter().take(count).enumerate() {
            let title = if r.title.trim().is_empty() {
                "Untitled"
            } else {
                r.title.trim()
            };
            lines.push(format!("{}. {}", i + 1, title));
            if let Some(src) = r.properties.as_ref().and_then(|p| p.url.as_deref()) {
                lines.push(format!("   Image: {}", src.trim()));
            }
            lines.push(format!("   Page: {}", r.url.trim()));
            if let Some(thumb) = r.thumbnail.as_ref().and_then(|t| t.src.as_deref()) {
                lines.push(format!("   Thumb: {}", thumb.trim()));
            }
            if let Some(src) = r.source.as_deref() {
                let src = src.trim();
                if !src.is_empty() {
                    lines.push(format!("   Source: {src}"));
                }
            }
        }
        Ok(ToolResult::success(lines.join("\n")))
    }
}

// ── Videos ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct VideoResp {
    #[serde(default)]
    results: Vec<VideoResult>,
}
#[derive(Debug, Deserialize)]
struct VideoResult {
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    age: Option<String>,
    #[serde(default)]
    video: Option<VideoMeta>,
}
#[derive(Debug, Deserialize)]
struct VideoMeta {
    #[serde(default)]
    duration: Option<String>,
    #[serde(default)]
    creator: Option<String>,
}

pub struct BraveVideoSearchTool {
    cfg: BraveConfig,
}

impl BraveVideoSearchTool {
    pub fn new(api_key: Option<String>, max_results: usize, timeout_secs: u64) -> Self {
        Self {
            cfg: BraveConfig {
                api_key,
                max_results: max_results.clamp(1, 20),
                timeout_secs: timeout_secs.max(1),
            },
        }
    }
}

#[async_trait]
impl Tool for BraveVideoSearchTool {
    fn name(&self) -> &str {
        "brave_video_search"
    }

    fn description(&self) -> &str {
        "Search for videos via Brave Search. Returns titles, URLs, creators, durations, and excerpts."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Video search query." },
                "count": { "type": "integer", "description": "Max results (1-20)." },
                "country": { "type": "string", "description": "2-letter country code." },
                "freshness": { "type": "string", "description": "Time filter: 'pd', 'pw', 'pm', 'py'." }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let query = extract_query(&args)?;
        let count = clamped_count(&args, self.cfg.max_results, 20);
        let mut q: Vec<(&str, String)> = vec![("q", query.clone()), ("count", count.to_string())];
        if let Some(c) = pub_string(&args, "country") {
            q.push(("country", c));
        }
        if let Some(f) = pub_string(&args, "freshness") {
            q.push(("freshness", f));
        }
        let raw = brave_get(&self.cfg, "/videos/search", &q).await?;
        let parsed: VideoResp = serde_json::from_value(raw)
            .map_err(|e| anyhow::anyhow!("Brave video response shape changed: {e}"))?;
        if parsed.results.is_empty() {
            return Ok(ToolResult::success(format!("No videos found for: {query}")));
        }
        let mut lines = vec![format!("Video results for: {query} (via Brave)")];
        for (i, r) in parsed.results.iter().take(count).enumerate() {
            let title = if r.title.trim().is_empty() {
                "Untitled"
            } else {
                r.title.trim()
            };
            lines.push(format!("{}. {}", i + 1, title));
            lines.push(format!("   {}", r.url.trim()));
            if let Some(meta) = r.video.as_ref() {
                if let Some(creator) = meta.creator.as_deref() {
                    let creator = creator.trim();
                    if !creator.is_empty() {
                        lines.push(format!("   Creator: {creator}"));
                    }
                }
                if let Some(dur) = meta.duration.as_deref() {
                    let dur = dur.trim();
                    if !dur.is_empty() {
                        lines.push(format!("   Duration: {dur}"));
                    }
                }
            }
            if let Some(age) = r.age.as_deref() {
                let age = age.trim();
                if !age.is_empty() {
                    lines.push(format!("   {age}"));
                }
            }
            let desc = r.description.trim();
            if !desc.is_empty() {
                lines.push(format!(
                    "   {}",
                    crate::openhuman::util::truncate_with_ellipsis(desc, 500)
                ));
            }
        }
        Ok(ToolResult::success(lines.join("\n")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn require_key_rejects_blank() {
        let cfg = BraveConfig {
            api_key: Some("   ".into()),
            max_results: 5,
            timeout_secs: 5,
        };
        assert!(cfg.require_key().is_err());
    }

    #[test]
    fn require_key_accepts_trimmed() {
        let cfg = BraveConfig {
            api_key: Some("  abc  ".into()),
            max_results: 5,
            timeout_secs: 5,
        };
        assert_eq!(cfg.require_key().unwrap(), "abc");
    }

    #[test]
    fn web_tool_advertises_unified_name() {
        let t = BraveWebSearchTool::new(Some("k".into()), 5, 5);
        assert_eq!(t.name(), "web_search_tool");
    }

    #[test]
    fn news_tool_name() {
        let t = BraveNewsSearchTool::new(Some("k".into()), 5, 5);
        assert_eq!(t.name(), "brave_news_search");
    }

    #[test]
    fn image_tool_name() {
        let t = BraveImageSearchTool::new(Some("k".into()), 5, 5);
        assert_eq!(t.name(), "brave_image_search");
    }

    #[test]
    fn video_tool_name() {
        let t = BraveVideoSearchTool::new(Some("k".into()), 5, 5);
        assert_eq!(t.name(), "brave_video_search");
    }

    #[tokio::test]
    async fn execute_without_key_returns_error() {
        let t = BraveWebSearchTool::new(None, 5, 5);
        let err = t
            .execute(json!({ "query": "test" }))
            .await
            .expect_err("should error without key");
        assert!(err.to_string().contains("no API key"));
    }

    #[test]
    fn clamped_count_respects_max() {
        assert_eq!(clamped_count(&json!({"count": 99}), 5, 20), 20);
        assert_eq!(clamped_count(&json!({"count": 0}), 5, 20), 1);
        assert_eq!(clamped_count(&json!({}), 5, 20), 5);
    }
}
