
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
