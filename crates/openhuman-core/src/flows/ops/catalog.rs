use super::*;

#[cfg(test)]
pub(crate) use crate::integrations::composio::catalog::toolkit_for_contract_slug;
pub(crate) use crate::integrations::composio::catalog::{lookup_contract, ContractLookupError};

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
    let tools = crate::integrations::composio::builder_tools::search_live_catalog(
        config, query, toolkit, limit,
    )
    .await;
    Ok(Outcome::single_log(
        json!({ "tools": tools }),
        "tool catalog searched",
    ))
}

/// Fetches one Composio action's full contract (secret-free) — the RPC the
/// canvas tool browser calls to fill in an action's arg schema, reusing the same
/// core as the agent's `get_tool_contract` tool.
pub async fn flows_get_tool_contract(
    config: &Config,
    slug: &str,
) -> Result<Outcome<Value>, String> {
    let trimmed = slug.trim();
    let (contract, runtime_gate) =
        lookup_contract(config, slug)
            .await
            .map_err(|error| match error {
                ContractLookupError::InvalidSlug { slug } => format!(
                    "Could not extract a toolkit from slug '{slug}' — it must look like \
             '<TOOLKIT>_<ACTION>' (e.g. 'GMAIL_SEND_EMAIL')."
                ),
                ContractLookupError::CatalogUnavailable { toolkit } => {
                    format!("Could not fetch the live Composio catalog for toolkit '{toolkit}'.")
                }
                ContractLookupError::ActionNotFound { slug, toolkit } => format!(
                    "'{slug}' is not a real action in the '{toolkit}' toolkit's live catalog."
                ),
            })?;
    let value = serde_json::to_value(&contract).map_err(|e| e.to_string())?;
    tracing::debug!(target: "flows", slug = %trimmed, toolkit = %contract.toolkit, "[flows] flows_get_tool_contract: fetching contract");
    let mut response = json!({ "contract": value });
    if let Some(warning) = runtime_gate {
        response["runtime_gate"] = json!(warning);
    }
    Ok(Outcome::single_log(response, "tool contract fetched"))
}
