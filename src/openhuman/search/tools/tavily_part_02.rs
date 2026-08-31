
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
        // from the scraps. Gate on the requested flag — the response can carry
        // an answer the agent never asked for.
        let include_answer = args
            .get("include_answer")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let answer = if include_answer {
            non_empty(parsed.answer.as_deref())
        } else {
            None
        };
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
