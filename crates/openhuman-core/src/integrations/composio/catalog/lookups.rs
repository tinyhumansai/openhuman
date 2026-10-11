//! Thin per-slug projections over [`super::contract::fetch_live_toolkit_catalog`]'s
//! live [`super::contract::ToolContract`]s: required-argument names and
//! response/output field names.

use super::contract::fetch_live_toolkit_catalog;
use crate::config::Config;

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
    let toolkit = crate::integrations::composio::contract::toolkit_from_slug(slug)?;
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
    let toolkit = crate::integrations::composio::contract::toolkit_from_slug(slug)?;
    let contracts = fetch_live_toolkit_catalog(config, &toolkit).await?;
    let contract = contracts
        .iter()
        .find(|c| c.slug.eq_ignore_ascii_case(slug))?;
    contract.output_schema.as_ref()?;
    Some(contract.output_fields.clone())
}
