
pub(crate) async fn push_observations(
    config: &Config,
    trace_ctx: &TraceContext,
    observations: &[AgentObservation],
    run_telemetry: Option<&RunTelemetry>,
) -> Result<(), String> {
    if observations.is_empty() {
        return Ok(());
    }
    let url = ingestion_url(config);
    // Same gate, same reasons as `push_spans` — both entry points are on the
    // per-turn path, so both skip before doing any work.
    let environment = environment_for_base(&url);
    if skip_push(environment) {
        return Ok(());
    }
    if !url.starts_with("http") {
        return Err(format!(
            "could not resolve Langfuse ingestion URL from backend host (got {url:?})"
        ));
    }
    let token = require_live_session_token(config)?;
    // Stamp the run lineage from the run's own observations so a spawned
    // sub-agent's trace links back to its parent turn (#4657).
    let trace_ctx = trace_ctx_with_run_lineage(trace_ctx, observations);
    let trace = trace_config_from_context(&trace_ctx, environment);
    let observation_count = observations.len();
    let observations = observations_for_export(&trace_ctx, observations);

    tracing::debug!(
        target: LOG_TARGET,
        "[agent-tracing] pushing {observation_count} journal observations to Langfuse at {url}"
    );

    let client = LangfuseClient::proxy(url, token)
        .map_err(|err| format!("Langfuse client setup failed: {err}"))?;
    let mut payload = client
        .build_ingestion_batch(trace, observations.as_ref())
        .map_err(|err| format!("Langfuse journal batch build failed: {err}"))?;
    if insert_run_telemetry_generation(&mut payload, run_telemetry) {
        tracing::debug!(
            target: LOG_TARGET,
            "[agent-tracing] added run telemetry aggregate to Langfuse journal batch"
        );
    } else {
        tracing::debug!(
            target: LOG_TARGET,
            "[agent-tracing] no run telemetry aggregate added to Langfuse journal batch"
        );
    }
    // Langfuse caps a single ingestion request at 500 events; a large run (e.g.
    // one that spawns sub-agents) can far exceed that and previously had its
    // ENTIRE trace rejected with a 400. Send in <=500-event chunks instead.
    for chunk in split_ingestion_batch(payload, LANGFUSE_MAX_BATCH_EVENTS) {
        tokio::time::timeout(PUSH_TIMEOUT, client.send_batch(chunk))
            .await
            .map_err(|_| format!("Langfuse journal push timed out after {PUSH_TIMEOUT:?}"))?
            .map_err(|err| format!("Langfuse journal ingestion failed: {err}"))?;
    }

    tracing::debug!(
        target: LOG_TARGET,
        "[agent-tracing] pushed {observation_count} journal observations to Langfuse"
    );
    Ok(())
}

/// Push `spans` to the co-hosted Langfuse server. Resolves the endpoint from the
/// current backend host and authenticates with the live session bearer. Returns
/// `Err` (for the caller to log + fall back) when there is no live session, the
/// host is unresolvable, the request fails, or Langfuse rejects the batch.
pub(crate) async fn push_spans(config: &Config, spans: &[TraceSpan]) -> Result<(), String> {
    if spans.is_empty() {
        return Ok(());
    }
    let url = ingestion_url(config);
    // Ahead of the URL check, the session lookup and the request: a skipped
    // environment must cost nothing per turn. An unresolvable URL lands in
    // `environment_for_base`'s catch-all, which is `production` — so a garbage
    // host skips rather than erroring, which is the right way round.
    let environment = environment_for_base(&url);
    if skip_push(environment) {
        return Ok(());
    }
    if !url.starts_with("http") {
        return Err(format!(
            "could not resolve Langfuse ingestion URL from backend host (got {url:?})"
        ));
    }
    let token = require_live_session_token(config)?;
    let include_content = config.observability.agent_tracing.capture_content;
    let batch = spans_to_langfuse_batch(spans, include_content, environment);
    let span_count = spans.len();

    tracing::debug!(
        target: LOG_TARGET,
        "[agent-tracing] pushing {span_count} spans to Langfuse at {url}"
    );

    // `ingestion_url` resolves to the backend's own Langfuse proxy route on the
    // backend host, authenticated with a TinyHumans session token — backend
    // traffic, so it carries the product identity. This is a bare
    // `reqwest::Client`, not `BackendOAuthClient`'s, so nothing is inherited
    // from that path's default headers; see [`crate::api::product`].
    let (product_header, product_value) = crate::api::product::product_identity_header();
    let response = reqwest::Client::new()
        .post(&url)
        .header(
            reqwest::header::AUTHORIZATION,
            bearer_authorization_value(&token),
        )
        .header(product_header, product_value)
        .timeout(PUSH_TIMEOUT)
        .json(&batch)
        .send()
        .await
        .map_err(|err| format!("POST {url} failed: {err}"))?;

    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        let excerpt: String = body.chars().take(200).collect();
        return Err(format!("Langfuse ingestion returned {status}: {excerpt}"));
    }
    // Langfuse returns 207 Multi-Status even when individual events are rejected
    // — the failures live in the response `errors` array, not the HTTP status.
    // Surface them (a partial rejection is logged but never fails the turn).
    let rejected = serde_json::from_str::<Value>(&body)
        .ok()
        .and_then(|v| v.get("errors").and_then(Value::as_array).cloned())
        .filter(|errs| !errs.is_empty());
    if let Some(errs) = rejected {
        let excerpt: String = serde_json::to_string(&errs)
            .unwrap_or_default()
            .chars()
            .take(400)
            .collect();
        tracing::warn!(
            target: LOG_TARGET,
            "[agent-tracing] Langfuse ({status}) rejected {} of {span_count} span event(s): {excerpt}",
            errs.len()
        );
    } else {
        tracing::debug!(
            target: LOG_TARGET,
            "[agent-tracing] pushed {span_count} spans to Langfuse ({status})"
        );
    }
    Ok(())
}
