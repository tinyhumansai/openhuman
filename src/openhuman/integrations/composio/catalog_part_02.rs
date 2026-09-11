
/// `get_tool_output_sample`'s implementation — see the module comment above
/// this section for why it exists. Gates, in order (fail closed on any):
///
/// 1. **Scope**: [`resolve_composio_action_scope`] must CONFIRM `slug` as
///    `Read` (`None` — no confirmed scope, e.g. an uncurated slug on a
///    cataloged toolkit — refuses exactly like a confirmed non-`Read` scope
///    does; it is never treated as "assume Read").
/// 2. **Connected**: the slug's toolkit must have an active Composio
///    connection.
///
/// On success, derives + caches a [`ProbedOutputSample`] (process-lifetime,
/// keyed by slug) and returns it. `args` is forwarded verbatim to the real
/// call — the builder should pass the SAME arguments it intends to wire into
/// the real `tool_call` node (this is a sample of THAT call, not a generic
/// fixture); omitted/`null` becomes `{}`, which is fine for a
/// zero-required-arg action.
pub(crate) async fn probe_tool_output_sample(
    config: &Config,
    slug: &str,
    args: Value,
) -> std::result::Result<ProbedOutputSample, String> {
    let slug = slug.trim();
    if slug.is_empty() {
        return Err("get_tool_output_sample: slug must not be empty".to_string());
    }

    match resolve_composio_action_scope(slug) {
        Some(crate::openhuman::integrations::composio::providers::ToolScope::Read) => {}
        Some(other) => {
            tracing::warn!(
                target: "flows",
                %slug,
                scope = other.as_str(),
                "[flows] get_tool_output_sample: refused — not a Read-scope action"
            );
            return Err(format!(
                "get_tool_output_sample refuses `{slug}`: classified as {} — this probe is \
                 READ-only and never performs a real mutation, regardless of the user's scope \
                 preference. Use get_tool_contract for its schema-derived (possibly unknown) \
                 output shape instead.",
                other.as_str()
            ));
        }
        None => {
            tracing::warn!(
                target: "flows",
                %slug,
                "[flows] get_tool_output_sample: refused — no confirmed Read scope (either no \
                 extractable toolkit, or an uncurated slug on a toolkit with a static curated \
                 catalog — fails closed rather than guessing via the verb heuristic)"
            );
            return Err(format!(
                "get_tool_output_sample refuses `{slug}`: could not confirm this is a Read-scope \
                 action. Either no toolkit could be extracted from the slug, or its toolkit ships \
                 a static curated catalog and this slug is not one of its curated actions — this \
                 probe never falls back to a verb-name heuristic in that case, since an uncurated \
                 action on a cataloged toolkit could really be a write. Use get_tool_contract for \
                 its schema-derived (possibly unknown) output shape instead."
            ));
        }
    }

    let Some(toolkit) =
        crate::openhuman::integrations::composio::providers::toolkit_from_slug(slug)
    else {
        return Err(format!(
            "get_tool_output_sample: could not extract a toolkit from slug '{slug}' — it must \
             look like '<TOOLKIT>_<ACTION>'."
        ));
    };

    let integrations =
        crate::openhuman::integrations::composio::fetch_connected_integrations(config).await;
    let connected = integrations
        .iter()
        .any(|i| i.connected && i.toolkit.eq_ignore_ascii_case(&toolkit));
    if !connected {
        tracing::warn!(target: "flows", %slug, %toolkit, "[flows] get_tool_output_sample: refused — toolkit not connected");
        return Err(format!(
            "get_tool_output_sample refuses `{slug}`: the '{toolkit}' toolkit has no active \
             Composio connection for this user — connect it first (composio_connect), or fall \
             back to get_tool_contract's schema-derived hint."
        ));
    }

    tracing::debug!(
        target: "flows",
        %slug,
        %toolkit,
        "[flows] get_tool_output_sample: probing the real live response (read-only, bounded, one call)"
    );

    let kind = create_composio_client(config).map_err(|e| e.to_string())?;
    let args_opt = if args.is_null() { None } else { Some(args) };
    let resp = match kind {
        ComposioClientKind::Backend(client) => client
            .execute_tool(slug, args_opt)
            .await
            .map_err(|e| format!("get_tool_output_sample: real call to `{slug}` failed: {e}"))?,
        ComposioClientKind::Direct(tool) => {
            direct_execute(&tool, slug, args_opt, &config.composio.entity_id, None)
                .await
                .map_err(|e| format!("get_tool_output_sample: real call to `{slug}` failed: {e}"))?
        }
    };

    if !resp.successful {
        let detail = resp
            .error
            .as_deref()
            .map(str::trim)
            .filter(|e| !e.is_empty())
            .unwrap_or("no error detail returned by the provider");
        return Err(format!(
            "get_tool_output_sample: `{slug}` reported failure at the connected provider: {detail}"
        ));
    }

    let envelope = serde_json::to_value(&resp).map_err(|e| {
        format!("get_tool_output_sample: could not serialize the real response: {e}")
    })?;
    let primary_array_path =
        compute_primary_array_path_from_value(&envelope, COMPOSIO_ENVELOPE_META_KEYS_AT_ROOT);
    let output_fields = resp
        .data
        .as_object()
        .map(|obj| obj.keys().cloned().collect())
        .unwrap_or_default();

    let sample = ProbedOutputSample {
        primary_array_path,
        output_fields,
        sample: envelope,
    };
    cache_probe_result(slug, sample.clone());
    tracing::info!(
        target: "flows",
        %slug,
        primary_array_path = ?sample.primary_array_path,
        "[flows] get_tool_output_sample: probed + cached the real output shape"
    );
    Ok(sample)
}

/// Best-effort lookup of a Composio action's **required** top-level parameter
/// names — a thin projection over [`fetch_live_toolkit_catalog`]'s
/// [`ToolContract`]s (this used to run its own independent
/// `REQUIRED_ARGS_CACHE`-backed fetch; existing callers — the required-arg
/// preflight, `graph_wiring_warnings` — keep this exact signature).
///
/// Returns `None` when the schema is unavailable — unknown toolkit, client
/// construction failure, a failed/empty listing, or the slug isn't present
/// in the toolkit's live catalog — so callers can skip the preflight rather
/// than block execution on a catalog hiccup.
pub(crate) async fn composio_required_args(config: &Config, slug: &str) -> Option<Vec<String>> {
    let toolkit = crate::openhuman::integrations::composio::providers::toolkit_from_slug(slug)?;
    let contracts = fetch_live_toolkit_catalog(config, &toolkit).await?;
    contracts
        .iter()
        .find(|c| c.slug.eq_ignore_ascii_case(slug))
        .map(|c| c.required_args.clone())
}

/// Best-effort lookup of a Composio action's **response/output** top-level
/// field names — the output-side analogue of [`composio_required_args`],
/// now a thin projection over [`fetch_live_toolkit_catalog`]'s
/// [`ToolContract`]s (replaces the standalone `RESPONSE_FIELDS_CACHE`-backed
/// fetch; `search_tool_catalog`'s grounding keeps this exact signature).
///
/// Returns `None` when no output schema is known for the slug — unknown
/// toolkit, client construction failure, a failed/empty listing, the slug
/// isn't in the live catalog, or a real action whose listing doesn't
/// publish `output_parameters` — so callers degrade to "output shape
/// unknown" rather than blocking or guessing. `Some(vec![])` means the
/// schema was found but names no top-level properties.
pub(crate) async fn composio_response_fields(config: &Config, slug: &str) -> Option<Vec<String>> {
    let toolkit = crate::openhuman::integrations::composio::providers::toolkit_from_slug(slug)?;
    let contracts = fetch_live_toolkit_catalog(config, &toolkit).await?;
    let contract = contracts
        .iter()
        .find(|c| c.slug.eq_ignore_ascii_case(slug))?;
    contract.output_schema.as_ref()?;
    Some(contract.output_fields.clone())
}
