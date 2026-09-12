
fn handle_searxng_search(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let query = params
            .get("query")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .ok_or_else(|| "missing or empty `query`".to_string())?;
        let max_results = params
            .get("max_results")
            .and_then(Value::as_u64)
            .map(|n| n.clamp(1, SEARXNG_MAX_RESULTS as u64) as usize);
        let categories = optional_string_array(&params, "categories")?;
        let language = params
            .get("language")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);

        let config = config_rpc::load_config_with_timeout().await?;
        if !config.searxng.enabled {
            tracing::debug!("[rpc][tools.searxng_search] searxng disabled — rejecting");
            return Err(
                "SearXNG search is not enabled. Set searxng.enabled=true or OPENHUMAN_SEARXNG_ENABLED=true."
                    .to_string(),
            );
        }

        tracing::debug!(
            query_len = query.chars().count(),
            max_results = max_results.unwrap_or(config.searxng.max_results),
            category_count = categories.len(),
            has_language = language.is_some(),
            base_url = %config.searxng.base_url,
            "[rpc][tools.searxng_search] start"
        );

        let tool = crate::openhuman::search::tools::SearxngSearchTool::new(
            config.searxng.base_url.clone(),
            config.searxng.max_results,
            config.searxng.default_language.clone(),
            config.searxng.timeout_secs,
        );

        let response = tool
            .search(crate::openhuman::search::tools::SearxngSearchArgs {
                query,
                categories,
                language,
                max_results,
            })
            .await
            .map_err(|e| format!("searxng search failed: {e:#}"))?;

        let result_count = response.results.len();
        let payload = json!({
            "query": response.query,
            "results": response.results,
        });
        let log = vec![format!(
            "[rpc][tools.searxng_search] success results={result_count}"
        )];
        RpcOutcome::new(payload, log).into_cli_compatible_json()
    })
}

fn handle_apify_linkedin_scrape(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let profile_url = params
            .get("profile_url")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .ok_or_else(|| "missing or empty `profile_url`".to_string())?;

        let config = config_rpc::load_config_with_timeout().await?;
        let client = crate::openhuman::integrations::build_client(&config).ok_or_else(|| {
            "Apify scrape unavailable — no backend session token. Sign in first.".to_string()
        })?;

        let data = crate::openhuman::agent::learning::linkedin_enrichment::scrape_linkedin_profile(
            &client,
            &profile_url,
        )
        .await
        .map_err(|e| format!("Apify LinkedIn scrape failed: {e:#}"))?;

        let markdown =
            crate::openhuman::agent::learning::linkedin_enrichment::render_profile_markdown(
                &profile_url,
                &data,
            );

        let payload = json!({ "data": data, "markdown": markdown });
        let log = vec![format!(
            "tools.apify_linkedin_scrape: url={profile_url} markdown_chars={}",
            markdown.chars().count()
        )];
        RpcOutcome::new(payload, log).into_cli_compatible_json()
    })
}

fn optional_string_array(params: &Map<String, Value>, key: &str) -> Result<Vec<String>, String> {
    let Some(value) = params.get(key) else {
        return Ok(Vec::new());
    };
    if value.is_null() {
        return Ok(Vec::new());
    }
    let items = value
        .as_array()
        .ok_or_else(|| format!("`{key}` must be an array of strings"))?;
    items
        .iter()
        .filter_map(|item| match item.as_str() {
            Some(value) => {
                let trimmed = value.trim();
                (!trimmed.is_empty()).then(|| Ok(trimmed.to_string()))
            }
            None => Some(Err(format!("`{key}` must contain only strings"))),
        })
        .collect()
}
