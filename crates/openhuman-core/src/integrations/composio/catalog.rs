//! Live Composio tool contracts, read from the `tinyconnectors` module.
//!
//! The module owns the fetch, the 30-minute cache, the confirmed-Read output
//! probe and the probe overlay; this file is the host's call surface over its
//! `GetToolContract` / `ListToolContracts` / `ProbeToolOutput` members. The
//! contract and probe types are the `tinyconnectors_bus` ones.
//!
//! A failed fetch is `None` here, as it always was: callers degrade to
//! "catalog unknown" rather than blocking on a catalog hiccup.

use crate::config::Config;
use serde_json::Value;
use tinyconnectors_bus::names::methods;
use tinyconnectors_bus::{
    ComposioListToolContractsRequest, ComposioListToolContractsResponse,
    ComposioProbeToolOutputRequest,
};

pub use tinyconnectors_bus::ComposioProbedOutputSample as ProbedOutputSample;
pub use tinyconnectors_bus::ComposioToolContract as ToolContract;

/// Shared authoring lookup failure, kept distinct so callers can preserve
/// their established user-facing error messages.
pub(crate) enum ContractLookupError {
    InvalidSlug { slug: String },
    CatalogUnavailable { toolkit: String },
    ActionNotFound { slug: String, toolkit: String },
}

/// Warning shared by both flow authoring surfaces when an action exists but
/// the runtime's curated allowlist will reject it.
pub(crate) const RUNTIME_GATE_WARNING: &str = "This action will be REJECTED on every real run — the runtime tool gate only allows curated actions for this toolkit. Pick a `featured: true` result from search_tool_catalog instead.";

/// Resolves the live contract and runtime-curation warning for a flow action.
pub(crate) async fn lookup_contract(
    config: &Config,
    slug: &str,
) -> Result<(ToolContract, Option<&'static str>), ContractLookupError> {
    let trimmed = slug.trim();
    let Some(toolkit) = toolkit_for_contract_slug(trimmed) else {
        return Err(ContractLookupError::InvalidSlug {
            slug: slug.to_string(),
        });
    };
    let Some(catalog) = fetch_live_toolkit_catalog(config, &toolkit).await else {
        return Err(ContractLookupError::CatalogUnavailable { toolkit });
    };
    let Some(contract) = catalog.iter().find(|contract| {
        crate::integrations::composio::contract::action_slug_matches(&contract.slug, trimmed)
    }) else {
        return Err(ContractLookupError::ActionNotFound {
            slug: trimmed.to_string(),
            toolkit,
        });
    };
    let warning =
        (!contract.is_curated && has_curated_catalog(&toolkit)).then_some(RUNTIME_GATE_WARNING);
    Ok((contract.clone(), warning))
}

/// Strictly extracts the toolkit from an action-shaped slug.
pub(crate) fn toolkit_for_contract_slug(slug: &str) -> Option<String> {
    let trimmed = slug.trim();
    let (toolkit_segment, action_segment) = trimmed.split_once('_')?;
    if toolkit_segment.is_empty() || action_segment.is_empty() {
        return None;
    }
    crate::integrations::composio::contract::toolkit_from_slug(trimmed)
}

/// Whether the toolkit has a product-curated catalog.
pub(crate) fn has_curated_catalog(toolkit: &str) -> bool {
    crate::integrations::composio::contract::catalog_for_toolkit(toolkit).is_some()
}

#[cfg(test)]
#[path = "catalog_seed.rs"]
mod seed;
#[cfg(test)]
pub(crate) use seed::{seed_live_catalog_cache, seed_live_catalog_cache_expired, seed_probe_cache};

#[cfg(test)]
#[path = "catalog_tests.rs"]
mod tests;

/// Every real action Composio publishes for `toolkit`, with any cached probe
/// already overlaid by the module. `None` on any failure.
pub(crate) async fn fetch_live_toolkit_catalog(
    config: &Config,
    toolkit: &str,
) -> Option<Vec<ToolContract>> {
    #[cfg(test)]
    if let Some(seeded) = seed::seeded_catalog(toolkit) {
        return Some(seeded);
    }
    let toolkit = toolkit.trim().to_ascii_lowercase();
    match super::module_client::call::<_, ComposioListToolContractsResponse>(
        config,
        methods::LIST_TOOL_CONTRACTS,
        ComposioListToolContractsRequest {
            toolkit: toolkit.clone(),
        },
    )
    .await
    {
        Ok(resp) => Some(resp.contracts),
        Err(error) => {
            tracing::debug!(target: "flows", %toolkit, %error,
                "[composio][catalog] ListToolContracts failed — catalog unknown");
            None
        }
    }
}

/// One bounded, confirmed-Read real call whose response shape the module
/// derives and caches. Refusals come back as the module's message.
pub(crate) async fn probe_tool_output_sample(
    config: &Config,
    slug: &str,
    args: Value,
) -> Result<ProbedOutputSample, String> {
    super::module_client::call::<_, ProbedOutputSample>(
        config,
        methods::PROBE_TOOL_OUTPUT,
        ComposioProbeToolOutputRequest {
            slug: slug.trim().to_string(),
            args,
        },
    )
    .await
}

/// A Composio action's **required** top-level parameter names, or `None` when
/// the schema is unavailable (so callers skip the preflight).
pub(crate) async fn composio_required_args(config: &Config, slug: &str) -> Option<Vec<String>> {
    let toolkit = super::contract::toolkit_from_slug(slug)?;
    let contracts = fetch_live_toolkit_catalog(config, &toolkit).await?;
    contracts
        .iter()
        .find(|c| c.slug.eq_ignore_ascii_case(slug))
        .map(|c| c.required_args.clone())
}

/// A Composio action's response top-level field names. `None` when no output
/// schema is known; `Some(vec![])` when one is known and names no properties.
pub(crate) async fn composio_response_fields(config: &Config, slug: &str) -> Option<Vec<String>> {
    let toolkit = super::contract::toolkit_from_slug(slug)?;
    let contracts = fetch_live_toolkit_catalog(config, &toolkit).await?;
    let contract = contracts
        .iter()
        .find(|c| c.slug.eq_ignore_ascii_case(slug))?;
    contract.output_schema.as_ref()?;
    Some(contract.output_fields.clone())
}
