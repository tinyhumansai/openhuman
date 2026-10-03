use super::*;

// ─────────────────────────────────────────────────────────────────────────────
// Catalog RPCs for the UI (Phase 5, item 16) — one implementation, two consumers
// ─────────────────────────────────────────────────────────────────────────────

/// Searches the live Composio tool catalog (secret-free) — the RPC the in-canvas
/// tool browser calls, reusing the exact same core as the agent's
/// `search_tool_catalog` tool so the two can't drift.
pub async fn flows_search_tool_catalog(
    config: &Config,
    query: &str,
    toolkit: Option<&str>,
    limit: usize,
) -> Result<Outcome<Value>, String> {
    tracing::debug!(target: "flows", %query, toolkit = toolkit.unwrap_or("<all>"), "[flows] flows_search_tool_catalog: searching live catalog");
    let tools =
        crate::flows::builder_tools::search_live_catalog(config, query, toolkit, limit).await;
    Ok(Outcome::single_log(
        json!({ "tools": tools }),
        "tool catalog searched",
    ))
}

/// Resolves the toolkit for a *single action* slug, rejecting anything that is
/// not shaped `<TOOLKIT>_<ACTION>`.
///
/// [`crate::integrations::composio::contract::toolkit_from_slug`] falls back to the whole
/// string when there is no `_`, so it answers `Some` for every non-empty input
/// — `toolkit_from_slug("nodashhere") == Some("nodashhere")`. That permissive
/// fall-back is load-bearing for `compute_required_connections`, which maps
/// toolkit-ish tokens and legitimately wants the whole string back, so the
/// stricter rule belongs to this caller rather than to the shared helper.
///
/// A contract fetch returns *one action*, so a slug with no action segment
/// cannot name anything it could return; rejecting it here also spares the
/// caller a pointless catalog round trip (which takes a per-toolkit fetch lock)
/// before failing with an unrelated "could not fetch the catalog" message.
///
/// The multi-segment toolkit prefixes (`MICROSOFT_TEAMS_`, `ONE_DRIVE_`,
/// `ZOHO_MAIL_`) all end in `_`, so a real action under one of them always has
/// non-empty segments either side of its first `_` and passes unchanged.
pub(crate) fn toolkit_for_contract_slug(slug: &str) -> Option<String> {
    let trimmed = slug.trim();
    let (toolkit_segment, action_segment) = trimmed.split_once('_')?;
    if toolkit_segment.is_empty() || action_segment.is_empty() {
        return None;
    }
    crate::integrations::composio::contract::toolkit_from_slug(trimmed)
}

/// Fetches one Composio action's full contract (secret-free) — the RPC the
/// canvas tool browser calls to fill in an action's arg schema, reusing the same
/// core as the agent's `get_tool_contract` tool.
pub async fn flows_get_tool_contract(
    config: &Config,
    slug: &str,
) -> Result<Outcome<Value>, String> {
    let trimmed = slug.trim();
    // Shape-check before `toolkit_from_slug`, and before any I/O: the shared
    // helper is deliberately permissive, so an unshaped slug would otherwise
    // fall through to a catalog round trip and fail with an unrelated message.
    // The message quotes the caller's own slug, not `trimmed` — reporting the
    // trimmed form named an empty string back at whoever sent whitespace.
    let Some(toolkit) = toolkit_for_contract_slug(trimmed) else {
        return Err(format!(
            "Could not extract a toolkit from slug '{slug}' — it must look like \
             '<TOOLKIT>_<ACTION>' (e.g. 'GMAIL_SEND_EMAIL')."
        ));
    };
    tracing::debug!(target: "flows", slug = %trimmed, %toolkit, "[flows] flows_get_tool_contract: fetching contract");
    let Some(catalog) =
        crate::flows::tinyflows::caps::fetch_live_toolkit_catalog(config, &toolkit).await
    else {
        return Err(format!(
            "Could not fetch the live Composio catalog for toolkit '{toolkit}'."
        ));
    };
    match catalog
        .iter()
        .find(|c| c.slug.eq_ignore_ascii_case(trimmed))
    {
        Some(contract) => {
            let contract = crate::flows::tinyflows::caps::apply_probe_override(contract.clone());
            let value = serde_json::to_value(&contract).map_err(|e| e.to_string())?;
            Ok(Outcome::single_log(
                json!({ "contract": value }),
                "tool contract fetched",
            ))
        }
        None => Err(format!(
            "'{trimmed}' is not a real action in the '{toolkit}' toolkit's live catalog."
        )),
    }
}
