use super::*;
use serde::{Deserialize, Serialize};

/// Secret-free OpenHuman picker projection for Composio accounts and named
/// HTTP credentials. This is the product RPC shape; secrets remain in their
/// domain stores and are injected only by CapsHost during execution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FlowConnection {
    /// Value placed on a workflow node's `connection_ref`.
    pub connection_ref: String,
    /// Source kind: `composio` or `http`.
    pub kind: String,
    /// Human-readable label.
    pub display: String,
    /// Composio toolkit slug, when applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub toolkit: Option<String>,
    /// HTTP credential scheme, when applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheme: Option<String>,
    /// Non-secret platform identity associated with a Composio connection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform_user_id: Option<String>,
}

/// Lists the connection sources a flow node's `connection_ref` can attach to:
/// Composio connected accounts (`kind = "composio"`) and stored HTTP
/// credentials (`kind = "http"`). This is the picker source for the Workflows
/// UI (and the agent's flow-authoring surface) — it returns ids + display
/// labels + kind ONLY, never any secret material.
///
/// The two sources are aggregated independently and are individually
/// fault-tolerant: a transient Composio backend/network failure (or an
/// unconfigured Direct-mode key) yields zero Composio entries but still returns
/// the HTTP credential half, and vice-versa. A failure in one source never
/// fails the whole picker.
pub async fn flows_list_connections(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
) -> Result<Outcome<Vec<FlowConnection>>, String> {
    tracing::debug!(
        "[flows] rpc flows_list_connections: aggregating composio + http_cred picker sources"
    );
    let mut logs = Vec::new();

    // 1. Composio connected accounts. Direct mode without a configured key
    //    already short-circuits to an empty list (a valid setup state, not an
    //    error); a backend outage returns Err — tolerate it so the picker still
    //    surfaces HTTP credentials.
    let composio_conns =
        match crate::integrations::composio::ops::composio_list_connections(ctx, config).await {
            Ok(outcome) => {
                tracing::debug!(
                    count = outcome.value.connections.len(),
                    "[flows] flows_list_connections: composio source returned connections"
                );
                outcome.value.connections
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "[flows] flows_list_connections: composio source unavailable — \
                     returning http_cred entries only"
                );
                logs.push(format!(
                    "flows_list_connections: composio source unavailable ({e})"
                ));
                Vec::new()
            }
        };

    // 2. Named HTTP credentials — secret-free summaries (the store never hands
    //    out secret material here; injection happens server-side in
    //    `flows::caps_host::OpenHumanHttp`).
    let http_creds =
        match crate::flows::http_credentials::HttpCredentialsStore::from_config(config).list() {
            Ok(list) => {
                tracing::debug!(
                    count = list.len(),
                    "[flows] flows_list_connections: http_cred store returned summaries"
                );
                list
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "[flows] flows_list_connections: http_cred store read failed — \
                     returning composio entries only"
                );
                logs.push(format!(
                    "flows_list_connections: http_cred store unavailable ({e})"
                ));
                Vec::new()
            }
        };

    // Connected-account identities (email/handle/platform user id), synced
    // via each toolkit's whoami-style call (e.g. Slack `SLACK_TEST_AUTH`) on
    // connection sync. Loaded once here so `build_flow_connections` can stay
    // a pure, unit-testable matcher.
    let identities =
        crate::integrations::composio::identity_store::load_connected_identities(ctx, config)
            .await
            .unwrap_or_else(|error| {
                tracing::warn!(
                    %error,
                    "[flows] flows_list_connections: load_connected_identities failed"
                );
                Vec::new()
            });
    tracing::debug!(
        count = identities.len(),
        "[flows] flows_list_connections: identity-cache load"
    );
    let keys = normalize_keys(config, &composio_conns, &identities).await;
    let connections = build_flow_connections(composio_conns, http_creds, &identities, &keys);
    tracing::debug!(
        total = connections.len(),
        "[flows] flows_list_connections: aggregated picker sources"
    );
    logs.push(format!(
        "flows_list_connections: {} connection(s)",
        connections.len()
    ));
    Ok(Outcome::new(connections, logs))
}

/// Normalise every connection key `build_flow_connections` matches on, in one
/// connector-module call. Returns an empty map when the module is unavailable;
/// the picker then lists connections without a `platform_user_id`.
async fn normalize_keys(
    config: &Config,
    composio: &[crate::integrations::composio::ComposioConnection],
    identities: &[crate::integrations::composio::contract::ConnectedIdentity],
) -> std::collections::HashMap<String, String> {
    let mut raw: Vec<String> = identities
        .iter()
        .flat_map(|id| [id.source.clone(), id.identifier.clone()])
        .collect();
    for conn in composio {
        raw.push(conn.normalized_toolkit());
        raw.push(conn.id.clone());
    }
    raw.sort();
    raw.dedup();
    if identities.is_empty() {
        return std::collections::HashMap::new();
    }
    match crate::integrations::composio::contract::normalize_connection_identifiers(config, &raw)
        .await
    {
        Ok(normalized) if normalized.len() == raw.len() => {
            raw.into_iter().zip(normalized).collect()
        }
        other => {
            tracing::debug!(
                error = ?other.err(),
                "[flows] flows_list_connections: key normalisation unavailable"
            );
            std::collections::HashMap::new()
        }
    }
}

/// Fold Composio connected accounts + named HTTP credentials into the flat,
/// secret-free [`FlowConnection`] picker list. Only ACTIVE Composio connections
/// are surfaced — a pending/expired OAuth account cannot execute a tool, so it
/// would be a dead pick. Pure (no I/O) so the aggregation shape is
/// unit-testable without a live backend; `identities` is loaded once by the
/// caller and matched in here.
///
/// Each Composio connection is also matched against `identities` (keyed by
/// `(toolkit, connection_id)`, both normalized the same way
/// `enrich_connections_with_identity` in `composio::ops::connections` does)
/// to attach `platform_user_id` — the connected account's own member id
/// (e.g. Slack `U123ABC`). This is what lets the workflow builder wire a
/// self-targeted action ("DM me") to the user's own account instead of
/// guessing a public channel.
pub(crate) fn build_flow_connections(
    composio: Vec<crate::integrations::composio::ComposioConnection>,
    http: Vec<crate::flows::http_credentials::HttpCredentialSummary>,
    identities: &[crate::integrations::composio::contract::ConnectedIdentity],
    keys: &std::collections::HashMap<String, String>,
) -> Vec<FlowConnection> {
    let entries =
        crate::integrations::composio::ops::flow_connection_entries(&composio, identities, keys);
    let mut out = Vec::with_capacity(composio.len() + http.len());
    out.extend(entries.into_iter().map(|entry| FlowConnection {
        connection_ref: entry.connection_ref,
        kind: "composio".to_string(),
        display: entry.display,
        toolkit: Some(entry.toolkit),
        scheme: None,
        platform_user_id: entry.platform_user_id,
    }));
    for cred in http {
        out.push(FlowConnection {
            // The `http_cred:<name>` value is the picker wire convention.
            connection_ref: format!("http_cred:{}", cred.name),
            kind: "http".to_string(),
            display: http_credential_display(&cred),
            toolkit: None,
            scheme: Some(cred.scheme),
            platform_user_id: None,
        });
    }
    out
}

/// Human-readable picker label for a named HTTP credential, e.g.
/// `"stripe (bearer)"`. Only the (non-secret) name + scheme — never the value.
fn http_credential_display(cred: &crate::flows::http_credentials::HttpCredentialSummary) -> String {
    format!("{} ({})", cred.name, cred.scheme)
}

#[cfg(test)]
pub(super) use crate::integrations::composio::ops::title_case_toolkit;

// ─────────────────────────────────────────────────────────────────────────────
// Connector onboarding (Phase 5, item 18) — which toolkits a graph needs
// ─────────────────────────────────────────────────────────────────────────────

/// The set of Composio toolkits currently connected (lowercased), derived from
/// the same picker source the node-config credential dropdown uses.
pub(crate) async fn connected_toolkits(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
) -> std::collections::HashSet<String> {
    match flows_list_connections(ctx, config).await {
        Ok(outcome) => outcome
            .value
            .iter()
            .filter_map(|c| c.toolkit.as_deref())
            .map(|t| t.to_ascii_lowercase())
            .collect(),
        Err(e) => {
            tracing::warn!(target: "flows", error = %e, "[flows] connected_toolkits: could not list connections — treating all as unconnected");
            std::collections::HashSet::new()
        }
    }
}

/// The Composio toolkits a graph needs (from its `tool_call` slugs and any
/// `app_event` trigger), each tagged connected/missing — the data behind the
/// canvas/proposal "Connect <toolkit>" CTAs (audit Phase 5, item 18). Native
/// `oh:` tools and `http_request` nodes need no Composio connection and are
/// skipped.
pub async fn compute_required_connections(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    graph: &Value,
) -> Vec<Value> {
    use crate::integrations::composio::contract::toolkit_from_slug;

    // Collect required toolkits (deduped, order-preserving).
    let mut required: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut push = |tk: String| {
        let tk = tk.to_ascii_lowercase();
        if !tk.is_empty() && seen.insert(tk.clone()) {
            required.push(tk);
        }
    };

    for node in crate::flows::host_gates::nodes(graph) {
        if node.kind == "tool_call" {
            if let Some(slug) = node.config.get("slug").and_then(Value::as_str) {
                // Native OpenHuman tools (`oh:<name>`) need no connection.
                if slug.starts_with("oh:") {
                    continue;
                }
                if let Some(tk) = toolkit_from_slug(slug) {
                    push(tk.to_string());
                }
            }
        }
    }
    // An app_event trigger names its toolkit directly.
    if let Some(trigger) =
        crate::flows::host_gates::nodes(graph).find(|node| node.kind == "trigger")
    {
        if let Some(tk) = trigger.config.get("toolkit").and_then(Value::as_str) {
            push(tk.to_string());
        }
    }

    if required.is_empty() {
        return Vec::new();
    }

    let connected = connected_toolkits(ctx, config).await;
    required
        .into_iter()
        .map(|toolkit| {
            let status = if connected.contains(&toolkit) {
                "connected"
            } else {
                "missing"
            };
            json!({ "toolkit": toolkit, "status": status })
        })
        .collect()
}

/// RPC: compute the toolkits a candidate graph needs and their connected
/// status, so the canvas/proposal can render "Connect <toolkit>" CTAs.
pub async fn flows_required_connections(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    graph_json: Value,
) -> Result<Outcome<Value>, String> {
    let graph = super::validation::migrate_over_bus(ctx, config, graph_json).await?;
    let required = compute_required_connections(ctx, config, &graph).await;
    Ok(Outcome::single_log(
        json!({ "required_connections": required }),
        "required connections computed",
    ))
}
