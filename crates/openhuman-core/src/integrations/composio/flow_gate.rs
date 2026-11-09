//! Composio policy used by flow stages and capability adapters.
//!
//! This module owns the host's Composio-specific decisions. It accepts plain
//! product inputs and returns ordinary values so it stays independent of the
//! flow engine implementation.

use serde_json::Value;
use tinyagents_harness::tool::missing_required_args;

use crate::config::Config;

/// Extracts a connection id from a `composio:<toolkit>:<id>` reference.
pub(crate) fn composio_connection_id(conn: &str) -> Option<&str> {
    let rest = conn.strip_prefix("composio:")?;
    let id = rest.rsplit(':').next()?;
    (!id.is_empty()).then_some(id)
}

/// Whether a slug needs a live connected-toolkit set to be authorized.
pub(crate) fn slug_needs_connected_set(slug: &str) -> bool {
    use crate::integrations::composio::contract::{catalog_for_toolkit, toolkit_from_slug};
    toolkit_from_slug(slug).is_some_and(|toolkit| catalog_for_toolkit(&toolkit).is_none())
}

/// Current active connected toolkits, with last-known cache fallback on outage.
pub(crate) async fn connected_toolkit_slugs(config: &Config) -> Option<Vec<String>> {
    use crate::integrations::composio::{
        cached_active_integrations, fetch_connected_integrations_status,
        FetchConnectedIntegrationsStatus,
    };

    let integrations = match fetch_connected_integrations_status(config).await {
        FetchConnectedIntegrationsStatus::Authoritative(items) => items,
        FetchConnectedIntegrationsStatus::Unavailable => {
            match cached_active_integrations(config).await {
                Some(items) => {
                    tracing::warn!(target: "flows", "[flows] connected-toolkit lookup: backend unavailable — using last-known (possibly stale) cached connections for the tool_call allowlist");
                    items
                }
                None => {
                    tracing::warn!(target: "flows", "[flows] connected-toolkit lookup: backend unavailable and no cached snapshot — connected-toolkit allowlist is empty this call");
                    return None;
                }
            }
        }
    };
    Some(
        integrations
            .into_iter()
            .filter(|item| item.connected)
            .map(|item| item.toolkit.to_ascii_lowercase())
            .collect(),
    )
}

/// Deny-by-default Composio flow-tool policy, including user scope preference.
pub(crate) async fn flow_tool_allowed(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    slug: &str,
    connected_toolkits: Option<&[String]>,
) -> bool {
    use crate::integrations::composio::contract::{
        catalog_for_toolkit, classify_unknown, find_curated, toolkit_from_slug,
    };
    use crate::integrations::composio::ops::load_user_scope_pref;

    let Some(toolkit) = toolkit_from_slug(slug) else {
        tracing::debug!(target: "flows", %slug, "[flows] tool_call curation: reject — slug has no extractable toolkit prefix");
        return false;
    };
    if let Some(catalog) = catalog_for_toolkit(&toolkit) {
        let Some(curated) = find_curated(&catalog, slug) else {
            tracing::debug!(target: "flows", %slug, %toolkit, "[flows] tool_call curation: reject — slug is not a curated action of this toolkit");
            return false;
        };
        // A pref that cannot be read admits nothing (fail closed).
        let allowed = load_user_scope_pref(ctx, config, &toolkit)
            .await
            .is_ok_and(|pref| pref.allows(curated.scope));
        tracing::debug!(target: "flows", %slug, %toolkit, allowed, "[flows] tool_call curation: static curated catalog decision");
        return allowed;
    }

    if !connected_toolkits.is_some_and(|toolkits| {
        toolkits
            .iter()
            .any(|item| item.eq_ignore_ascii_case(&toolkit))
    }) {
        tracing::debug!(target: "flows", %slug, %toolkit, "[flows] tool_call curation: reject — toolkit has no static catalog and is not connected");
        return false;
    }
    let Some(live_catalog) =
        crate::integrations::composio::catalog::fetch_live_toolkit_catalog(config, &toolkit).await
    else {
        tracing::warn!(target: "flows", %slug, %toolkit, "[flows] tool_call curation: reject — connected but the live catalog fetch failed (fail-closed)");
        return false;
    };
    if !live_catalog
        .iter()
        .any(|item| item.slug.eq_ignore_ascii_case(slug))
    {
        tracing::debug!(target: "flows", %slug, %toolkit, "[flows] tool_call curation: reject — slug is not a real action in this toolkit's live catalog");
        return false;
    }
    let allowed = load_user_scope_pref(ctx, config, &toolkit)
        .await
        .is_ok_and(|pref| pref.allows(classify_unknown(slug)));
    tracing::debug!(target: "flows", %slug, %toolkit, allowed, "[flows] tool_call curation: live catalog + scope decision");
    allowed
}

/// Applies the connected-set lookup only for toolkits without a static catalog.
pub(crate) async fn is_curated_flow_tool(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    slug: &str,
) -> bool {
    let connected = if slug_needs_connected_set(slug) {
        connected_toolkit_slugs(config).await
    } else {
        None
    };
    flow_tool_allowed(ctx, config, slug, connected.as_deref()).await
}

/// Classifies a curated action for the autonomy-tier gate.
pub(crate) fn classify_composio_action_for_tier(
    slug: &str,
) -> crate::security::policy::CommandClass {
    use crate::integrations::composio::contract::{curated_scope_for, ToolScope};
    use crate::security::policy::CommandClass;
    match curated_scope_for(slug) {
        Some(ToolScope::Read) => CommandClass::Read,
        Some(ToolScope::Write) | Some(ToolScope::Admin) | None => CommandClass::Network,
    }
}

/// Resolves a connection id to its toolkit and display label from a snapshot.
pub(crate) fn resolve_account<'a>(
    integrations: &'a [crate::integrations::composio::ConnectedIntegration],
    connection_id: &str,
) -> Option<(&'a str, Option<&'a str>)> {
    integrations.iter().find_map(|integration| {
        integration
            .connections
            .iter()
            .find(|connection| connection.connection_id == connection_id)
            .map(|connection| (integration.toolkit.as_str(), connection.label.as_deref()))
    })
}

/// Best-effort account lookup used for safe execution logging.
pub(crate) async fn resolve_composio_account(
    config: &Config,
    connection_id: &str,
) -> Option<(String, Option<String>)> {
    let integrations = crate::integrations::composio::fetch_connected_integrations(config).await;
    resolve_account(&integrations, connection_id)
        .map(|(toolkit, label)| (toolkit.to_string(), label.map(str::to_string)))
}

/// Runs the product-owned static and live-catalog required-argument checks.
pub(crate) async fn preflight_composio_args(
    config: &Config,
    slug: &str,
    args: &Value,
) -> Result<(), String> {
    if let Err(error) = crate::integrations::composio::processing::prepare(
        config,
        slug,
        Some(args.clone()),
        None,
        None,
    )
    .await
    {
        tracing::warn!(target: "flows", %slug, error = %error, "[flows] preflight: static arg rule rejected the call — failing before dispatch");
        return Err(format!("tool_call `{slug}`: {error}"));
    }
    let Some(required) =
        crate::integrations::composio::catalog::composio_required_args(config, slug).await
    else {
        tracing::info!(target: "flows", %slug, "[flows] preflight: no live catalog schema for action — required-arg check limited to static rules");
        return Ok(());
    };
    let missing = missing_required_args(&required, args);
    if missing.is_empty() {
        tracing::debug!(target: "flows", %slug, "[flows] preflight: all required args present");
        return Ok(());
    }
    tracing::warn!(target: "flows", %slug, ?missing, "[flows] preflight: required arg(s) missing or null — failing before dispatch");
    let list = missing
        .iter()
        .map(|name| format!("`{name}`"))
        .collect::<Vec<_>>()
        .join(", ");
    let first = &missing[0];
    Err(format!("tool_call `{slug}`: required arg(s) {list} missing or resolved to null — wire each from an upstream node's output, e.g. \"{first}\": \"=nodes.<node_id>.item.json.<field>\" (drop `.json` only if `<node_id>` is a code/transform/split_out/merge/trigger node — `agent`/`tool_call`/`http_request` nodes wrap their output in a `{{json,text,raw}}` envelope). If the value comes from an agent node, give that agent an output schema (config.output_parser.schema) so its fields are addressable."))
}

/// Converts a provider-side unsuccessful execution response into a policy error.
pub(crate) fn reject_unsuccessful_composio_response(
    slug: &str,
    response: crate::integrations::composio::ComposioExecuteResponse,
) -> Result<crate::integrations::composio::ComposioExecuteResponse, String> {
    if response.successful {
        return Ok(response);
    }
    let detail = response
        .error
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("no error detail returned by the provider");
    Err(format!(
        "tool_call `{slug}` failed at the connected provider: {detail}"
    ))
}

#[cfg(test)]
#[path = "flow_gate_tests.rs"]
mod tests;
