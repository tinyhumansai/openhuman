//! Composio contract validation that can be shared by flow authoring stages.

use serde_json::Value;
use tinyagents_harness::tool::{missing_required_args, unsupported_arg_names};

/// A graph node reduced to the Composio fields needed by the connection gate.
#[derive(Debug, Clone)]
pub(crate) struct ConnectionRefNode {
    pub id: String,
    pub slug: String,
    pub connection_ref: String,
}

/// A live account reduced to the fields used by the connection gate.
#[derive(Debug, Clone)]
pub(crate) struct ConnectedAccountRef {
    pub connection_ref: String,
    pub toolkit: Option<String>,
}

/// A flow tool call reduced to the fields needed for contract validation.
#[derive(Debug, Clone)]
pub(crate) struct ToolContractNode {
    pub id: String,
    pub slug: String,
    pub args: Value,
}

/// Parses a `composio:<toolkit>:<connection_id>` reference.
pub(crate) fn parse_composio_connection_ref(conn_ref: &str) -> Option<(&str, &str)> {
    let rest = conn_ref.strip_prefix("composio:")?;
    let (toolkit, id) = rest.rsplit_once(':')?;
    if toolkit.trim().is_empty() || id.trim().is_empty() {
        return None;
    }
    Some((toolkit.trim(), id.trim()))
}

fn first_connection_ref_for_toolkit<'a>(
    connections: &'a [ConnectedAccountRef],
    toolkit: &str,
) -> Option<&'a str> {
    connections
        .iter()
        .find(|connection| {
            connection
                .toolkit
                .as_deref()
                .is_some_and(|value| value.eq_ignore_ascii_case(toolkit))
        })
        .map(|connection| connection.connection_ref.as_str())
}

/// Validates connection shape, toolkit match and (when available) account id.
/// `None` connections means live lookup failed and preserves fail-open behavior.
pub(crate) fn validate_connection_refs(
    nodes: &[ConnectionRefNode],
    connections: Option<&[ConnectedAccountRef]>,
) -> Vec<String> {
    use crate::integrations::composio::contract::toolkit_from_slug;

    let mut errors = Vec::new();
    for node in nodes {
        let slug = node.slug.as_str();
        if slug.starts_with('=') || slug.starts_with("oh:") || node.connection_ref.trim().is_empty()
        {
            continue;
        }
        let Some(slug_toolkit) = toolkit_from_slug(slug) else {
            continue;
        };
        let conn_ref = node.connection_ref.as_str();
        let Some((ref_toolkit, ref_id)) = parse_composio_connection_ref(conn_ref) else {
            errors.push(format!(
                "Node '{}': `connection_ref` `{conn_ref}` is malformed — a Composio account ref \
                 must look like `composio:<toolkit>:<connection_id>` (e.g. \
                 `composio:{slug_toolkit}:<id>`). Call list_flow_connections and copy a \
                 `connection_ref` value verbatim.",
                node.id
            ));
            continue;
        };
        if !ref_toolkit.eq_ignore_ascii_case(&slug_toolkit) {
            let hint = match connections.and_then(|items| first_connection_ref_for_toolkit(items, &slug_toolkit)) {
                Some(reference) => format!(" — did you mean `{reference}`?"),
                None => format!(" — no `{slug_toolkit}` account is connected; connect one with composio_connect (or ask the user to), then use its `connection_ref`"),
            };
            errors.push(format!(
                "Node '{}': `connection_ref` `{conn_ref}` names the `{ref_toolkit}` toolkit but the \
                 tool_call slug `{slug}` is a `{slug_toolkit}` action{hint}.",
                node.id
            ));
            continue;
        }
        let Some(connections) = connections else {
            continue;
        };
        let id_exists = connections.iter().any(|connection| {
            connection
                .toolkit
                .as_deref()
                .is_some_and(|toolkit| toolkit.eq_ignore_ascii_case(&slug_toolkit))
                && parse_composio_connection_ref(&connection.connection_ref)
                    .is_some_and(|(_, id)| id.eq_ignore_ascii_case(ref_id))
        });
        if id_exists {
            continue;
        }
        match first_connection_ref_for_toolkit(connections, &slug_toolkit) {
            Some(reference) => errors.push(format!(
                "Node '{}': `connection_ref` `{conn_ref}` does not match any connected \
                 `{slug_toolkit}` account — did you mean `{reference}`? Call list_flow_connections and \
                 copy a `connection_ref` value verbatim.",
                node.id
            )),
            None => errors.push(format!(
                "Node '{}': `connection_ref` `{conn_ref}` names a `{slug_toolkit}` account, but \
                 no `{slug_toolkit}` account is connected — connect one with composio_connect \
                 (or ask the user to), then use its `connection_ref`.",
                node.id
            )),
        }
    }
    errors
}

/// Checks live action existence, curated runtime eligibility, and its args.
pub(crate) async fn validate_tool_contract_nodes(
    config: &crate::config::Config,
    nodes: &[ToolContractNode],
) -> Vec<String> {
    use crate::integrations::composio::{catalog, contract};

    let mut errors = Vec::new();
    for node in nodes {
        let slug = node.slug.as_str();
        if slug.starts_with('=') || slug.starts_with("oh:") {
            continue;
        }
        let Some(toolkit) = contract::toolkit_from_slug(slug) else {
            continue;
        };
        let Some(actions) = catalog::fetch_live_toolkit_catalog(config, &toolkit).await else {
            tracing::debug!(target: "flows", node = %node.id, %slug, %toolkit, "[flows] tool-contract check: live catalog fetch failed — skipping (best-effort, never false-rejects)");
            continue;
        };
        let Some(action) = actions
            .iter()
            .find(|item| contract::action_slug_matches(&item.slug, slug))
        else {
            tracing::warn!(target: "flows", node = %node.id, %slug, %toolkit, "[flows] tool-contract check: slug is not a real action in the live catalog — rejecting");
            errors.push(format!(
                "Node '{}': `{slug}` is not a real action in the `{toolkit}` toolkit's live \
                 Composio catalog — use search_tool_catalog {{ query: ..., toolkit: \"{toolkit}\" \
                 }} to find a real action slug.",
                node.id
            ));
            continue;
        };
        if catalog::has_curated_catalog(&toolkit) && !action.is_curated {
            tracing::warn!(target: "flows", node = %node.id, %slug, %toolkit, "[flows] tool-contract check: slug is real but not curated for a statically-catalogued toolkit — rejecting to match the runtime allowlist");
            errors.push(format!(
                "Node '{}': `{slug}` is a real `{toolkit}` action but not one of OpenHuman's \
                 curated actions for `{toolkit}` — the runtime tool gate only allows curated \
                 actions for toolkits with a curated catalog, so this would be rejected on \
                 every run. Use search_tool_catalog {{ query: ..., toolkit: \"{toolkit}\" }} and \
                 pick a result with `featured: true`.",
                node.id
            ));
            continue;
        }
        let missing = missing_required_args(&action.required_args, &node.args);
        if !missing.is_empty() {
            tracing::warn!(target: "flows", node = %node.id, %slug, ?missing, "[flows] tool-contract check: required arg(s) missing or null — rejecting");
            let list = missing
                .iter()
                .map(|name| format!("`{name}`"))
                .collect::<Vec<_>>()
                .join(", ");
            errors.push(format!(
                "Node '{}': tool_call `{slug}` is missing required arg(s) {list} — wire each \
                 from an upstream node's output, e.g. \"{}\": \
                 \"=nodes.<node_id>.item.json.<field>\" (call get_tool_contract {{ slug: \
                 \"{slug}\" }} for the exact required_args list).",
                node.id, missing[0]
            ));
        }
        if let Some(unsupported) = unsupported_arg_names(action.input_schema.as_ref(), &node.args) {
            if !unsupported.is_empty() {
                let mut valid_names = action
                    .input_schema
                    .as_ref()
                    .and_then(|schema| schema.get("properties"))
                    .and_then(Value::as_object)
                    .map(|properties| properties.keys().cloned().collect::<Vec<_>>())
                    .unwrap_or_default();
                valid_names.sort();
                tracing::warn!(target: "flows", node = %node.id, %slug, ?unsupported, ?valid_names, "[flows] tool-contract check: arg name(s) not declared by the action's input schema — rejecting");
                let bad_list = unsupported
                    .iter()
                    .map(|name| format!("`{name}`"))
                    .collect::<Vec<_>>()
                    .join(", ");
                let valid_suffix = if valid_names.is_empty() {
                    String::new()
                } else {
                    format!(
                        " — valid arg names for `{slug}` are: {}",
                        valid_names.join(", ")
                    )
                };
                errors.push(format!(
                    "Node '{}': tool_call `{slug}` has unsupported arg name(s) {bad_list} — not \
                     a property of this action's input schema{valid_suffix}. Call \
                     get_tool_contract {{ slug: \"{slug}\" }} and use the exact property names \
                     from `input_schema` (never guess an arg name).",
                    node.id
                ));
            }
        }
    }
    errors
}

#[cfg(test)]
#[path = "validation_tests.rs"]
mod tests;
