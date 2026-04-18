//! RPC-facing operations for the Composio domain.
//!
//! Each `composio_*` function wraps a [`ComposioClient`] call, translates
//! errors to strings, and returns an [`RpcOutcome`] so the controller
//! schemas can log a user-visible line. The handlers in [`super::schemas`]
//! call into these.
//!
//! These ops are also callable directly from other domains (e.g. the
//! agent harness) when they need composio data at runtime.

use crate::openhuman::config::Config;
use crate::rpc::RpcOutcome;

/// Result alias used by every `composio_*` op in this module.
///
/// We deliberately return a plain `String` error instead of
/// `anyhow::Error` — the controller layer in `schemas.rs` forwards
/// these straight into the RPC envelope, and `String` keeps the shape
/// obvious at a glance.
type OpResult<T> = std::result::Result<T, String>;

use std::sync::Arc;

use super::client::{build_composio_client, ComposioClient};
use super::providers::{
    get_provider, ProviderContext, ProviderUserProfile, SyncOutcome, SyncReason,
};
use super::types::{
    ComposioAuthorizeResponse, ComposioConnectionsResponse, ComposioCreateTriggerResponse,
    ComposioDeleteResponse, ComposioExecuteResponse, ComposioGithubReposResponse,
    ComposioToolkitsResponse, ComposioToolsResponse, ComposioTriggerHistoryResult,
};

/// Resolve a [`ComposioClient`] from the root config, or return an
/// error string that the caller can surface over RPC.
///
/// Composio is always enabled — it is proxied through our backend and
/// has no client-side toggle or API key. The only reason this fails is
/// that no app-session JWT has been stored yet (i.e. the user hasn't
/// completed sign-in / `auth_store_session`).
fn resolve_client(config: &Config) -> OpResult<ComposioClient> {
    build_composio_client(config).ok_or_else(|| {
        "composio unavailable: no backend session token. Sign in first \
         (auth_store_session)."
            .to_string()
    })
}

// ── Toolkits ────────────────────────────────────────────────────────

pub async fn composio_list_toolkits(
    config: &Config,
) -> OpResult<RpcOutcome<ComposioToolkitsResponse>> {
    tracing::debug!("[composio] rpc list_toolkits");
    let client = resolve_client(config)?;
    let resp = client
        .list_toolkits()
        .await
        .map_err(|e| format!("[composio] list_toolkits failed: {e:#}"))?;
    let count = resp.toolkits.len();
    Ok(RpcOutcome::new(
        resp,
        vec![format!("composio: {count} toolkit(s) enabled")],
    ))
}

// ── Connections ─────────────────────────────────────────────────────

pub async fn composio_list_connections(
    config: &Config,
) -> OpResult<RpcOutcome<ComposioConnectionsResponse>> {
    tracing::debug!("[composio] rpc list_connections");
    let client = resolve_client(config)?;
    let resp = client
        .list_connections()
        .await
        .map_err(|e| format!("[composio] list_connections failed: {e:#}"))?;
    let active = resp
        .connections
        .iter()
        .filter(|c| matches!(c.status.as_str(), "ACTIVE" | "CONNECTED"))
        .count();
    let total = resp.connections.len();
    Ok(RpcOutcome::new(
        resp,
        vec![format!(
            "composio: {total} connection(s) listed ({active} active)"
        )],
    ))
}

pub async fn composio_authorize(
    config: &Config,
    toolkit: &str,
) -> OpResult<RpcOutcome<ComposioAuthorizeResponse>> {
    tracing::debug!(toolkit = %toolkit, "[composio] rpc authorize");
    let client = resolve_client(config)?;
    let resp = client
        .authorize(toolkit)
        .await
        .map_err(|e| format!("[composio] authorize failed: {e:#}"))?;

    // Publish an event so any interested subscribers (e.g. UI refreshers,
    // analytics) can react to the new connection handoff.
    crate::core::event_bus::publish_global(
        crate::core::event_bus::DomainEvent::ComposioConnectionCreated {
            toolkit: toolkit.to_string(),
            connection_id: resp.connection_id.clone(),
            connect_url: resp.connect_url.clone(),
        },
    );

    Ok(RpcOutcome::new(
        resp,
        vec![format!("composio: authorize flow started for {toolkit}")],
    ))
}

pub async fn composio_delete_connection(
    config: &Config,
    connection_id: &str,
) -> OpResult<RpcOutcome<ComposioDeleteResponse>> {
    tracing::debug!(connection_id = %connection_id, "[composio] rpc delete_connection");
    let client = resolve_client(config)?;
    let resp = client
        .delete_connection(connection_id)
        .await
        .map_err(|e| format!("[composio] delete_connection failed: {e:#}"))?;
    // Bust the integrations cache so the next prompt reflects the removal.
    invalidate_connected_integrations_cache();
    Ok(RpcOutcome::new(
        resp,
        vec![format!("composio: connection {connection_id} deleted")],
    ))
}

// ── Tools ───────────────────────────────────────────────────────────

pub async fn composio_list_tools(
    config: &Config,
    toolkits: Option<Vec<String>>,
) -> OpResult<RpcOutcome<ComposioToolsResponse>> {
    tracing::debug!(?toolkits, "[composio] rpc list_tools");
    let client = resolve_client(config)?;
    let resp = client
        .list_tools(toolkits.as_deref())
        .await
        .map_err(|e| format!("[composio] list_tools failed: {e:#}"))?;
    let count = resp.tools.len();
    Ok(RpcOutcome::new(
        resp,
        vec![format!("composio: {count} tool(s) listed")],
    ))
}

// ── Execute ─────────────────────────────────────────────────────────

pub async fn composio_execute(
    config: &Config,
    tool: &str,
    arguments: Option<serde_json::Value>,
) -> OpResult<RpcOutcome<ComposioExecuteResponse>> {
    tracing::debug!(tool = %tool, "[composio] rpc execute");
    let client = resolve_client(config)?;
    let started = std::time::Instant::now();
    let result = client.execute_tool(tool, arguments.clone()).await;
    let elapsed_ms = started.elapsed().as_millis() as u64;

    match result {
        Ok(mut resp) => {
            crate::core::event_bus::publish_global(
                crate::core::event_bus::DomainEvent::ComposioActionExecuted {
                    tool: tool.to_string(),
                    success: resp.successful,
                    error: resp.error.clone(),
                    cost_usd: resp.cost_usd,
                    elapsed_ms,
                },
            );
            // Mirror the agent-tool path (see `tools::ComposioExecuteTool::execute`):
            // route through the toolkit's native provider so CLI and JSON-RPC
            // callers see the same envelope the agent sees (e.g. Gmail HTML →
            // markdown). `raw_html: true` in `arguments` opts out for
            // `GMAIL_FETCH_EMAILS`.
            //
            // Provider registry is populated by `bus::start_composio_bus` on
            // the server path; the CLI/RPC one-shot path never boots the bus,
            // so ensure the built-ins are registered before we look up. The
            // init fn is idempotent.
            if resp.successful {
                super::providers::init_default_providers();
                if let Some(toolkit) = super::providers::toolkit_from_slug(tool) {
                    if let Some(provider) = super::providers::get_provider(&toolkit) {
                        tracing::trace!(
                            toolkit = toolkit.as_str(),
                            tool = tool,
                            has_args = arguments.is_some(),
                            "[composio] post-processing action result"
                        );
                        provider.post_process_action_result(
                            tool,
                            arguments.as_ref(),
                            &mut resp.data,
                        );
                    }
                }
            }
            Ok(RpcOutcome::new(
                resp,
                vec![format!("composio: executed {tool} ({elapsed_ms}ms)")],
            ))
        }
        Err(e) => {
            crate::core::event_bus::publish_global(
                crate::core::event_bus::DomainEvent::ComposioActionExecuted {
                    tool: tool.to_string(),
                    success: false,
                    error: Some(e.to_string()),
                    cost_usd: 0.0,
                    elapsed_ms,
                },
            );
            Err(format!("[composio] execute failed: {e:#}"))
        }
    }
}

// ── GitHub repos + trigger provisioning ─────────────────────────────

pub async fn composio_list_github_repos(
    config: &Config,
    connection_id: Option<String>,
) -> OpResult<RpcOutcome<ComposioGithubReposResponse>> {
    tracing::debug!(?connection_id, "[composio] rpc list_github_repos");
    let client = resolve_client(config)?;
    let resp = client
        .list_github_repos(connection_id.as_deref())
        .await
        .map_err(|e| format!("[composio] list_github_repos failed: {e:#}"))?;
    let count = resp.repositories.len();
    let connection_id = resp.connection_id.clone();
    Ok(RpcOutcome::new(
        resp,
        vec![format!(
            "composio: {count} github repo(s) listed for connection {connection_id}"
        )],
    ))
}

pub async fn composio_create_trigger(
    config: &Config,
    slug: &str,
    connection_id: Option<String>,
    trigger_config: Option<serde_json::Value>,
) -> OpResult<RpcOutcome<ComposioCreateTriggerResponse>> {
    tracing::debug!(slug = %slug, ?connection_id, "[composio] rpc create_trigger");
    let client = resolve_client(config)?;
    let resp = client
        .create_trigger(slug, connection_id.as_deref(), trigger_config)
        .await
        .map_err(|e| format!("[composio] create_trigger failed: {e:#}"))?;
    let trigger_id = resp.trigger_id.clone();
    Ok(RpcOutcome::new(
        resp,
        vec![format!(
            "composio: trigger {trigger_id} created for slug {slug}"
        )],
    ))
}

// ── Trigger history ────────────────────────────────────────────────

pub async fn composio_list_trigger_history(
    config: &Config,
    limit: Option<usize>,
) -> OpResult<RpcOutcome<ComposioTriggerHistoryResult>> {
    let requested_limit = limit.unwrap_or(100).clamp(1, 500);
    let workspace_label = config
        .workspace_dir
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("<workspace>");
    tracing::debug!(
        limit = requested_limit,
        workspace = workspace_label,
        "[composio] rpc list_trigger_history"
    );

    let store = super::trigger_history::global().ok_or_else(|| {
        "[composio] trigger history unavailable: archive store is not initialized".to_string()
    })?;

    let history = store
        .list_recent(requested_limit)
        .map_err(|error| format!("[composio] list_trigger_history failed: {error}"))?;
    let count = history.entries.len();

    Ok(RpcOutcome::new(
        history,
        vec![format!(
            "composio: {count} trigger history entrie(s) loaded (archive present)"
        )],
    ))
}

// ── Provider-backed ops ─────────────────────────────────────────────
//
// `composio_get_user_profile` and `composio_sync` route through the
// per-toolkit `ComposioProvider` registry instead of executing a
// single Composio action directly. The caller passes a `connection_id`,
// the op resolves the connection's toolkit slug from the backend, looks
// up the provider, and dispatches to it.
//
// These exist because individual toolkits need to do *several*
// `composio.execute` calls + bespoke result reshaping to produce a
// usable user profile or sync snapshot — wrapping that in a single
// RPC method keeps the UI/agent surface tiny and consistent across
// toolkits.

/// Look up the toolkit slug for an existing connection. Returns an
/// error string if the connection is unknown to the backend.
async fn resolve_toolkit_for_connection(
    client: &ComposioClient,
    connection_id: &str,
) -> OpResult<String> {
    tracing::debug!(connection_id = %connection_id, "[composio] resolve_toolkit_for_connection");
    let resp = client
        .list_connections()
        .await
        .map_err(|e| format!("[composio] list_connections failed: {e:#}"))?;
    let conn = resp
        .connections
        .into_iter()
        .find(|c| c.id == connection_id)
        .ok_or_else(|| format!("[composio] no connection with id '{connection_id}'"))?;
    Ok(conn.toolkit)
}

/// `openhuman.composio_get_user_profile` — fetch a normalized user
/// profile for a connected account by dispatching to the toolkit's
/// registered [`super::providers::ComposioProvider`].
pub async fn composio_get_user_profile(
    config: &Config,
    connection_id: &str,
) -> OpResult<RpcOutcome<ProviderUserProfile>> {
    tracing::debug!(connection_id = %connection_id, "[composio] rpc get_user_profile");
    let client = resolve_client(config)?;
    let toolkit = resolve_toolkit_for_connection(&client, connection_id).await?;

    let provider = get_provider(&toolkit).ok_or_else(|| {
        format!("[composio] no native provider registered for toolkit '{toolkit}'")
    })?;

    let ctx = ProviderContext {
        config: Arc::new(config.clone()),
        client,
        toolkit: toolkit.clone(),
        connection_id: Some(connection_id.to_string()),
    };

    let profile = provider
        .fetch_user_profile(&ctx)
        .await
        .map_err(|e| format!("[composio] get_user_profile({toolkit}) failed: {e}"))?;

    // Side-effect: persist profile fields into the local user_profile
    // facet table so any RPC call also refreshes the local store.
    let facets = super::providers::profile::persist_provider_profile(&profile);
    tracing::debug!(
        toolkit = %toolkit,
        facets_written = facets,
        "[composio] profile facets persisted from get_user_profile"
    );

    Ok(RpcOutcome::new(
        profile,
        vec![format!(
            "composio: fetched {toolkit} profile for connection {connection_id}"
        )],
    ))
}

/// `openhuman.composio_sync` — run a sync pass for a connected account
/// by dispatching to the toolkit's registered provider. `reason` is
/// `"manual"` by default; the periodic scheduler passes `"periodic"`
/// and the OAuth event subscriber passes `"connection_created"`.
pub async fn composio_sync(
    config: &Config,
    connection_id: &str,
    reason: Option<String>,
) -> OpResult<RpcOutcome<SyncOutcome>> {
    let reason = parse_sync_reason(reason.as_deref())?;
    tracing::debug!(
        connection_id = %connection_id,
        reason = reason.as_str(),
        "[composio] rpc sync"
    );
    let client = resolve_client(config)?;
    let toolkit = resolve_toolkit_for_connection(&client, connection_id).await?;

    let provider = get_provider(&toolkit).ok_or_else(|| {
        format!("[composio] no native provider registered for toolkit '{toolkit}'")
    })?;

    let ctx = ProviderContext {
        config: Arc::new(config.clone()),
        client,
        toolkit: toolkit.clone(),
        connection_id: Some(connection_id.to_string()),
    };

    let outcome = provider
        .sync(&ctx, reason)
        .await
        .map_err(|e| format!("[composio] sync({toolkit}) failed: {e}"))?;

    let summary = outcome.summary.clone();
    Ok(RpcOutcome::new(outcome, vec![summary]))
}

/// Parse the optional `reason` parameter into a [`SyncReason`].
///
/// `None` and the explicit `"manual"` value both map to
/// [`SyncReason::Manual`]. Any other unrecognized string is rejected
/// with a clear error so a typo in a caller (UI, CLI, agent) surfaces
/// at the RPC boundary instead of being silently coerced.
fn parse_sync_reason(raw: Option<&str>) -> OpResult<SyncReason> {
    match raw {
        None | Some("manual") => Ok(SyncReason::Manual),
        Some("periodic") => Ok(SyncReason::Periodic),
        Some("connection_created") => Ok(SyncReason::ConnectionCreated),
        Some(other) => Err(format!(
            "[composio] unrecognized sync reason '{other}': expected one of \
             'manual', 'periodic', 'connection_created'"
        )),
    }
}

// ── Prompt integration discovery ────────────────────────────────────

use crate::openhuman::context::prompt::{ConnectedIntegration, ConnectedIntegrationTool};
use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

/// Process-wide cache for connected integrations, keyed by the config
/// identity (the `config_path` string) so different user contexts don't
/// collide. Each entry is populated on first fetch and returned on
/// subsequent calls until explicitly invalidated.
static INTEGRATIONS_CACHE: LazyLock<RwLock<HashMap<String, Vec<ConnectedIntegration>>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// Derive a stable cache key from a [`Config`]. We use the stringified
/// `config_path` because it uniquely identifies a user context (it
/// resolves to the per-user openhuman dir).
fn cache_key(config: &Config) -> String {
    config.config_path.display().to_string()
}

/// Clear cached connected integrations so the next call to
/// [`fetch_connected_integrations`] hits the backend again.
///
/// Called by [`super::bus::ComposioConnectionCreatedSubscriber`] when a
/// new OAuth connection completes, and can also be called from tests.
/// Clears the entire map because the bus subscriber doesn't carry a
/// config reference.
pub fn invalidate_connected_integrations_cache() {
    if let Ok(mut guard) = INTEGRATIONS_CACHE.write() {
        guard.clear();
        tracing::debug!("[composio] connected integrations cache invalidated");
    }
}

/// Fetch the user's active Composio connections and their available
/// tool actions, returning a prompt-ready summary.
///
/// This is the **single source of truth** for connected integration
/// data injected into system prompts — both the agent turn loop and
/// the debug dump CLI call this function.
///
/// Results are cached process-wide (keyed by config identity) and
/// returned instantly on subsequent calls. The cache is invalidated
/// when a new connection is created
/// (via [`invalidate_connected_integrations_cache`]) or on process
/// restart.
///
/// Best-effort: returns an empty vec when the user isn't signed in,
/// the backend is unreachable, or any step fails.
pub async fn fetch_connected_integrations(config: &Config) -> Vec<ConnectedIntegration> {
    let key = cache_key(config);

    // Fast path: return cached result.
    if let Ok(guard) = INTEGRATIONS_CACHE.read() {
        if let Some(cached) = guard.get(&key) {
            tracing::debug!(
                count = cached.len(),
                key = %key,
                "[composio] fetch_connected_integrations: returning cached result"
            );
            return cached.clone();
        }
    }

    match fetch_connected_integrations_uncached(config).await {
        Some(result) => {
            // Backend was reachable — cache the result (even if empty).
            if let Ok(mut guard) = INTEGRATIONS_CACHE.write() {
                guard.insert(key, result.clone());
            }
            result
        }
        None => {
            // No auth / client unavailable — do NOT cache so a
            // subsequent call with a different config can retry.
            Vec::new()
        }
    }
}

/// The actual backend fetch, called on cache miss.
///
/// Returns `Some(vec)` when the backend was reachable. The returned
/// vector is the merged **integration overview** — every toolkit in
/// the backend allowlist appears as one entry, with a `connected`
/// flag indicating whether the user has an active OAuth connection.
/// Connected entries also carry the per-action tool catalogue
/// (fetched in a single batched call).
///
/// Returns `None` when we couldn't even build a client (no auth),
/// signalling the caller should NOT cache this result.
async fn fetch_connected_integrations_uncached(
    config: &Config,
) -> Option<Vec<ConnectedIntegration>> {
    use super::providers::toolkit_description;

    let Some(client) = build_composio_client(config) else {
        tracing::debug!("[composio] fetch_connected_integrations: no client (not signed in?)");
        return None;
    };

    // Pull the backend allowlist — every toolkit the orchestrator can
    // possibly suggest, regardless of whether the user has authorized
    // it yet. This is the universe of valid `toolkit` arguments to
    // `spawn_subagent(integrations_agent, …)`.
    //
    // On transient backend errors we return `None` instead of a
    // degraded `Some(Vec::new())` so `fetch_connected_integrations`
    // does NOT cache the failure. Caching an empty allowlist would
    // hide every integration from the orchestrator until the process
    // restarts or the cache is explicitly invalidated — a single 5xx
    // during startup would silently break delegation for the whole
    // session.
    let allowlisted_toolkits: Vec<String> = match client.list_toolkits().await {
        Ok(resp) => resp.toolkits,
        Err(e) => {
            tracing::warn!("[composio] fetch_connected_integrations: list_toolkits failed: {e}");
            return None;
        }
    };

    if allowlisted_toolkits.is_empty() {
        tracing::debug!("[composio] fetch_connected_integrations: backend allowlist is empty");
        return Some(Vec::new());
    }

    let connections = match client.list_connections().await {
        Ok(resp) => resp.connections,
        Err(e) => {
            tracing::warn!("[composio] fetch_connected_integrations: list_connections failed: {e}");
            // Same rationale as above — caching a snapshot where
            // every toolkit is marked as not-connected would
            // silently wipe main's Delegation Guide's "available
            // now" bullets for the rest of the session.
            return None;
        }
    };

    // Active connection slugs (status filter mirrors the original logic).
    let connected_slugs: std::collections::HashSet<String> = connections
        .iter()
        .filter(|c| c.status == "ACTIVE" || c.status == "CONNECTED")
        .map(|c| c.toolkit.clone())
        .collect();

    // Fetch available tool schemas — only for the connected slugs,
    // since not-connected toolkits won't be invoked from a sub-agent.
    let connected_slugs_vec: Vec<String> = {
        let mut v: Vec<String> = connected_slugs.iter().cloned().collect();
        v.sort();
        v
    };
    let tools_by_toolkit = if connected_slugs_vec.is_empty() {
        Vec::new()
    } else {
        match client.list_tools(Some(&connected_slugs_vec)).await {
            Ok(resp) => resp.tools,
            Err(e) => {
                tracing::warn!("[composio] fetch_connected_integrations: list_tools failed: {e}");
                // Same rationale as list_toolkits/list_connections —
                // caching connected entries with empty `tools` vectors
                // would cause `subagent_runner::run_typed_mode` to
                // build zero dynamic Composio action tools for a
                // toolkit-scoped `integrations_agent` spawn, silently
                // leaving the sub-agent with nothing callable.
                return None;
            }
        }
    };

    // Deduplicate the allowlist so a backend that returns duplicates
    // doesn't produce dual entries downstream.
    let mut unique_toolkits: Vec<String> = allowlisted_toolkits.clone();
    unique_toolkits.sort();
    unique_toolkits.dedup();

    // Build one entry per allowlisted toolkit. Connected entries
    // carry their action catalogue; not-connected entries carry an
    // empty `tools` vec.
    let mut integrations: Vec<ConnectedIntegration> = Vec::with_capacity(unique_toolkits.len());
    for slug in &unique_toolkits {
        let connected = connected_slugs.contains(slug);
        // Anchor the prefix with an underscore so slugs that share
        // a text prefix (e.g. `git` vs `github`) don't false-match
        // each other's actions. `GMAIL_SEND_EMAIL` matches `gmail_`,
        // not just `gmail`, so siblings stay in their own buckets.
        let action_prefix = format!("{}_", slug.to_uppercase());
        let tools: Vec<ConnectedIntegrationTool> = if connected {
            // Apply the same curated-whitelist + user-scope filter the
            // meta-tool layer uses, so the integrations_agent prompt
            // only advertises actions the agent is actually allowed to
            // call. One pref load per toolkit (not per action).
            let pref = super::providers::load_user_scope_or_default(slug).await;
            let filtered: Vec<&super::types::ComposioToolSchema> = tools_by_toolkit
                .iter()
                .filter(|t| t.function.name.starts_with(&action_prefix))
                .filter(|t| super::providers::is_action_visible_with_pref(&t.function.name, &pref))
                .collect();
            tracing::debug!(
                toolkit = %slug,
                kept = filtered.len(),
                "[composio][scopes] integrations prompt action set"
            );
            filtered
                .into_iter()
                .map(|t| ConnectedIntegrationTool {
                    name: t.function.name.clone(),
                    description: t.function.description.clone().unwrap_or_default(),
                    parameters: t.function.parameters.clone(),
                })
                .collect()
        } else {
            Vec::new()
        };

        integrations.push(ConnectedIntegration {
            toolkit: slug.clone(),
            description: toolkit_description(slug).to_string(),
            tools,
            connected,
        });
    }

    integrations.sort_by(|a, b| a.toolkit.cmp(&b.toolkit));

    let connected_count = integrations.iter().filter(|i| i.connected).count();
    tracing::info!(
        total = integrations.len(),
        connected = connected_count,
        "[composio] fetch_connected_integrations: done"
    );
    for ci in &integrations {
        tracing::debug!(
            toolkit = %ci.toolkit,
            connected = ci.connected,
            tool_count = ci.tools.len(),
            "[composio] integration overview"
        );
    }

    Some(integrations)
}

/// Just-in-time fetch of every available action for a single Composio
/// toolkit, returned in the [`ConnectedIntegrationTool`] shape the
/// `integrations_agent` spawn path expects.
///
/// Unlike [`fetch_connected_integrations`] (which bulk-fetches every
/// connected toolkit's tools once per session and caches the result),
/// this helper is uncached and scoped to a single toolkit — meant to
/// be called at `integrations_agent` spawn time so the sub-agent's
/// prompt always reflects the toolkit's current action catalogue.
///
/// The filter `starts_with("{TOOLKIT}_")` matches
/// `fetch_connected_integrations_uncached`'s own namespacing rule so
/// siblings like `github` / `git` don't leak into each other's buckets.
///
/// Returns an empty vec when the backend has no actions for the
/// toolkit (valid steady state for a freshly-authorised integration
/// whose catalogue hasn't been published yet). Returns `Err` only for
/// transport / auth failures the caller should surface to the user.
pub async fn fetch_toolkit_actions(
    client: &ComposioClient,
    toolkit: &str,
) -> anyhow::Result<Vec<ConnectedIntegrationTool>> {
    let toolkit_slug = toolkit.trim();
    if toolkit_slug.is_empty() {
        anyhow::bail!("fetch_toolkit_actions: toolkit must not be empty");
    }
    tracing::debug!(toolkit = %toolkit_slug, "[composio] fetch_toolkit_actions");
    let resp = client
        .list_tools(Some(&[toolkit_slug.to_string()]))
        .await
        .map_err(|e| anyhow::anyhow!("list_tools failed for toolkit `{toolkit_slug}`: {e}"))?;
    let action_prefix = format!("{}_", toolkit_slug.to_uppercase());
    // Apply curated whitelist + user scope so spawn-time tool
    // discovery agrees with the bulk path and the meta-tool layer.
    let pref = super::providers::load_user_scope_or_default(toolkit_slug).await;
    let actions: Vec<ConnectedIntegrationTool> = resp
        .tools
        .into_iter()
        .filter(|t| t.function.name.starts_with(&action_prefix))
        .filter(|t| super::providers::is_action_visible_with_pref(&t.function.name, &pref))
        .map(|t| ConnectedIntegrationTool {
            name: t.function.name,
            description: t.function.description.unwrap_or_default(),
            parameters: t.function.parameters,
        })
        .collect();
    tracing::debug!(
        toolkit = %toolkit_slug,
        action_count = actions.len(),
        "[composio] fetch_toolkit_actions: done"
    );
    Ok(actions)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_sync_reason_accepts_known_values() {
        assert_eq!(parse_sync_reason(None).unwrap(), SyncReason::Manual);
        assert_eq!(
            parse_sync_reason(Some("manual")).unwrap(),
            SyncReason::Manual
        );
        assert_eq!(
            parse_sync_reason(Some("periodic")).unwrap(),
            SyncReason::Periodic
        );
        assert_eq!(
            parse_sync_reason(Some("connection_created")).unwrap(),
            SyncReason::ConnectionCreated
        );
    }

    #[test]
    fn parse_sync_reason_rejects_unknown_values() {
        let err = parse_sync_reason(Some("scheduled")).unwrap_err();
        assert!(err.contains("unrecognized sync reason"));
        assert!(err.contains("scheduled"));
        // Typo of a real value should also fail rather than coerce.
        assert!(parse_sync_reason(Some("Periodic")).is_err());
        assert!(parse_sync_reason(Some("")).is_err());
    }

    // ── resolve_client / ops auth errors ──────────────────────────

    fn test_config(tmp: &tempfile::TempDir) -> Config {
        let mut c = Config::default();
        c.workspace_dir = tmp.path().join("workspace");
        c.config_path = tmp.path().join("config.toml");
        c.api_key = None; // ensure no token fallback
        c
    }

    #[test]
    fn resolve_client_errors_without_session() {
        let tmp = tempfile::tempdir().unwrap();
        let config = test_config(&tmp);
        // `ComposioClient` intentionally doesn't implement `Debug` — use a
        // pattern match instead of `.unwrap_err()`.
        let Err(err) = resolve_client(&config) else {
            panic!("expected auth error when no session is stored");
        };
        assert!(err.contains("composio unavailable"));
        assert!(err.contains("auth_store_session"));
    }

    #[tokio::test]
    async fn composio_list_toolkits_errors_without_session() {
        let tmp = tempfile::tempdir().unwrap();
        let config = test_config(&tmp);
        let err = composio_list_toolkits(&config).await.unwrap_err();
        assert!(err.contains("composio unavailable"));
    }

    #[tokio::test]
    async fn composio_list_connections_errors_without_session() {
        let tmp = tempfile::tempdir().unwrap();
        let config = test_config(&tmp);
        let err = composio_list_connections(&config).await.unwrap_err();
        assert!(err.contains("composio unavailable"));
    }

    #[tokio::test]
    async fn composio_authorize_errors_without_session() {
        let tmp = tempfile::tempdir().unwrap();
        let config = test_config(&tmp);
        let err = composio_authorize(&config, "gmail").await.unwrap_err();
        assert!(err.contains("composio unavailable"));
    }

    #[tokio::test]
    async fn composio_delete_connection_errors_without_session() {
        let tmp = tempfile::tempdir().unwrap();
        let config = test_config(&tmp);
        let err = composio_delete_connection(&config, "c-1")
            .await
            .unwrap_err();
        assert!(err.contains("composio unavailable"));
    }

    #[tokio::test]
    async fn composio_list_tools_errors_without_session() {
        let tmp = tempfile::tempdir().unwrap();
        let config = test_config(&tmp);
        let err = composio_list_tools(&config, None).await.unwrap_err();
        assert!(err.contains("composio unavailable"));
    }

    #[tokio::test]
    async fn composio_execute_errors_without_session() {
        let tmp = tempfile::tempdir().unwrap();
        let config = test_config(&tmp);
        let err = composio_execute(&config, "GMAIL_SEND_EMAIL", None)
            .await
            .unwrap_err();
        assert!(err.contains("composio unavailable"));
    }

    #[tokio::test]
    async fn composio_get_user_profile_errors_without_session() {
        let tmp = tempfile::tempdir().unwrap();
        let config = test_config(&tmp);
        let err = composio_get_user_profile(&config, "c-1").await.unwrap_err();
        assert!(err.contains("composio unavailable"));
    }

    #[tokio::test]
    async fn composio_sync_errors_without_session() {
        let tmp = tempfile::tempdir().unwrap();
        let config = test_config(&tmp);
        let err = composio_sync(&config, "c-1", None).await.unwrap_err();
        assert!(err.contains("composio unavailable"));
    }

    #[tokio::test]
    async fn composio_sync_rejects_invalid_reason_before_client_check() {
        let tmp = tempfile::tempdir().unwrap();
        let config = test_config(&tmp);
        // Invalid reason → should fail at parse step *before* touching the
        // client, so the error message references the reason, not auth.
        let err = composio_sync(&config, "c-1", Some("weird".into()))
            .await
            .unwrap_err();
        assert!(err.contains("unrecognized sync reason"));
    }

    #[tokio::test]
    async fn composio_list_trigger_history_errors_when_store_not_init() {
        let tmp = tempfile::tempdir().unwrap();
        let config = test_config(&tmp);
        // The trigger history store is a process-global singleton. If
        // another test in the same binary already initialised it (e.g.
        // via the archive-roundtrip test), skip rather than asserting on
        // the uninitialised branch.
        if super::super::trigger_history::global().is_some() {
            return;
        }
        let err = composio_list_trigger_history(&config, Some(10))
            .await
            .unwrap_err();
        assert!(err.contains("archive store is not initialized"));
    }

    // ── cache_key / invalidate_connected_integrations_cache ───────

    #[test]
    fn cache_key_is_based_on_config_path_string() {
        let tmp = tempfile::tempdir().unwrap();
        let mut a = Config::default();
        a.config_path = tmp.path().join("a.toml");
        let mut b = Config::default();
        b.config_path = tmp.path().join("b.toml");
        assert_ne!(cache_key(&a), cache_key(&b));
        assert_eq!(cache_key(&a), cache_key(&a));
    }

    #[tokio::test]
    async fn fetch_connected_integrations_returns_empty_without_auth() {
        let tmp = tempfile::tempdir().unwrap();
        let config = test_config(&tmp);
        let integrations = fetch_connected_integrations(&config).await;
        assert!(integrations.is_empty());
    }

    #[test]
    fn invalidate_connected_integrations_cache_is_safe_without_prior_insert() {
        // Must not panic on an empty cache.
        invalidate_connected_integrations_cache();
        invalidate_connected_integrations_cache();
    }

    // ── Mock-backend integration tests for ops ─────────────────────

    use axum::{
        extract::{Path, Query},
        routing::{get, post},
        Json, Router,
    };
    use serde_json::{json, Value};
    use std::collections::HashMap;

    async fn start_mock_backend(app: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        // Wait until the axum accept loop is actually serving — not just
        // until the kernel-level TCP socket is bound. Without this, fast
        // tests can fire a request before `axum::serve` starts polling and
        // occasionally see connection resets / hangs on loaded CI.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let mut backoff = std::time::Duration::from_millis(2);
        loop {
            if tokio::net::TcpStream::connect(addr).await.is_ok() {
                break;
            }
            if std::time::Instant::now() >= deadline {
                panic!("mock backend at {addr} did not become ready in time");
            }
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(std::time::Duration::from_millis(50));
        }

        format!("http://127.0.0.1:{}", addr.port())
    }

    fn config_with_backend(tmp: &tempfile::TempDir, base: String) -> Config {
        let mut c = Config::default();
        c.workspace_dir = tmp.path().join("workspace");
        c.config_path = tmp.path().join("config.toml");
        c.api_key = Some("test-token".into());
        c.api_url = Some(base);
        c
    }

    #[tokio::test]
    async fn composio_list_toolkits_via_mock() {
        let app = Router::new().route(
            "/agent-integrations/composio/toolkits",
            get(|| async { Json(json!({"success": true, "data": {"toolkits": ["gmail"]}})) }),
        );
        let base = start_mock_backend(app).await;
        let tmp = tempfile::tempdir().unwrap();
        let config = config_with_backend(&tmp, base);
        let outcome = composio_list_toolkits(&config).await.unwrap();
        assert_eq!(outcome.value.toolkits, vec!["gmail".to_string()]);
        assert!(outcome.logs.iter().any(|l| l.contains("toolkit")));
    }

    #[tokio::test]
    async fn composio_list_connections_via_mock_counts_active() {
        let app = Router::new().route(
            "/agent-integrations/composio/connections",
            get(|| async {
                Json(json!({
                    "success": true,
                    "data": {"connections": [
                        {"id":"c1","toolkit":"gmail","status":"ACTIVE"},
                        {"id":"c2","toolkit":"notion","status":"PENDING"},
                        {"id":"c3","toolkit":"gmail","status":"CONNECTED"}
                    ]}
                }))
            }),
        );
        let base = start_mock_backend(app).await;
        let tmp = tempfile::tempdir().unwrap();
        let config = config_with_backend(&tmp, base);
        let outcome = composio_list_connections(&config).await.unwrap();
        assert_eq!(outcome.value.connections.len(), 3);
        // 2 active, 3 total
        assert!(outcome.logs.iter().any(|l| l.contains("3 connection")));
        assert!(outcome.logs.iter().any(|l| l.contains("2 active")));
    }

    #[tokio::test]
    async fn composio_authorize_via_mock_publishes_event_and_returns_url() {
        let app = Router::new().route(
            "/agent-integrations/composio/authorize",
            post(|Json(_b): Json<Value>| async move {
                Json(json!({
                    "success": true,
                    "data": {"connectUrl": "https://x", "connectionId": "c1"}
                }))
            }),
        );
        let base = start_mock_backend(app).await;
        let tmp = tempfile::tempdir().unwrap();
        let config = config_with_backend(&tmp, base);
        let outcome = composio_authorize(&config, "gmail").await.unwrap();
        assert_eq!(outcome.value.connect_url, "https://x");
        assert_eq!(outcome.value.connection_id, "c1");
    }

    #[tokio::test]
    async fn composio_delete_connection_via_mock() {
        let app = Router::new().route(
            "/agent-integrations/composio/connections/{id}",
            axum::routing::delete(|Path(_id): Path<String>| async move {
                Json(json!({"success": true, "data": {"deleted": true}}))
            }),
        );
        let base = start_mock_backend(app).await;
        let tmp = tempfile::tempdir().unwrap();
        let config = config_with_backend(&tmp, base);
        let outcome = composio_delete_connection(&config, "c1").await.unwrap();
        assert!(outcome.value.deleted);
    }

    #[tokio::test]
    async fn composio_list_tools_via_mock_with_filter() {
        let app = Router::new().route(
            "/agent-integrations/composio/tools",
            get(|Query(_q): Query<HashMap<String, String>>| async move {
                Json(json!({
                    "success": true,
                    "data": {"tools": [
                        {"type":"function","function":{"name":"GMAIL_SEND_EMAIL"}},
                        {"type":"function","function":{"name":"GMAIL_SEARCH"}}
                    ]}
                }))
            }),
        );
        let base = start_mock_backend(app).await;
        let tmp = tempfile::tempdir().unwrap();
        let config = config_with_backend(&tmp, base);
        let outcome = composio_list_tools(&config, Some(vec!["gmail".into()]))
            .await
            .unwrap();
        assert_eq!(outcome.value.tools.len(), 2);
    }

    #[tokio::test]
    async fn composio_execute_via_mock_succeeds_and_logs_elapsed() {
        let app = Router::new().route(
            "/agent-integrations/composio/execute",
            post(|Json(b): Json<Value>| async move {
                Json(json!({
                    "success": true,
                    "data": {
                        "data": {"echo": b["tool"]},
                        "successful": true,
                        "error": null,
                        "costUsd": 0.001
                    }
                }))
            }),
        );
        let base = start_mock_backend(app).await;
        let tmp = tempfile::tempdir().unwrap();
        let config = config_with_backend(&tmp, base);
        let outcome = composio_execute(&config, "GMAIL_SEND", Some(json!({"to": "a"})))
            .await
            .unwrap();
        assert!(outcome.value.successful);
        assert!(outcome
            .logs
            .iter()
            .any(|l| l.contains("executed GMAIL_SEND")));
    }

    #[tokio::test]
    async fn composio_execute_via_mock_propagates_backend_error() {
        let app = Router::new().route(
            "/agent-integrations/composio/execute",
            post(|| async { Json(json!({"success": false, "error": "rate limited"})) }),
        );
        let base = start_mock_backend(app).await;
        let tmp = tempfile::tempdir().unwrap();
        let config = config_with_backend(&tmp, base);
        let err = composio_execute(&config, "ANY_TOOL", None)
            .await
            .unwrap_err();
        assert!(err.contains("execute failed"));
    }

    #[tokio::test]
    async fn fetch_connected_integrations_via_mock_aggregates_tools() {
        // Connections: gmail + notion. Tools: filtered to those toolkits
        // and prefixed with the uppercased slug. The toolkits route
        // backs the `list_toolkits()` allowlist gate that
        // `fetch_connected_integrations_uncached` calls before touching
        // connections — without it the function bails out at the first
        // step and returns an empty vec.
        let app = Router::new()
            .route(
                "/agent-integrations/composio/toolkits",
                get(|| async {
                    Json(json!({
                        "success": true,
                        "data": {"toolkits": ["gmail", "notion"]}
                    }))
                }),
            )
            .route(
                "/agent-integrations/composio/connections",
                get(|| async {
                    Json(json!({
                        "success": true,
                        "data": {"connections": [
                            {"id":"c1","toolkit":"gmail","status":"ACTIVE"},
                            {"id":"c2","toolkit":"notion","status":"CONNECTED"}
                        ]}
                    }))
                }),
            )
            .route(
                "/agent-integrations/composio/tools",
                get(|| async {
                    Json(json!({
                        "success": true,
                        "data": {"tools": [
                            {"type":"function","function":{
                                "name":"GMAIL_SEND_EMAIL",
                                "description":"Send"
                            }},
                            {"type":"function","function":{
                                "name":"NOTION_CREATE_PAGE",
                                "description":"Create"
                            }}
                        ]}
                    }))
                }),
            );
        let base = start_mock_backend(app).await;
        let tmp = tempfile::tempdir().unwrap();
        // Use a fresh cache key by isolating config_path.
        let config = config_with_backend(&tmp, base);
        invalidate_connected_integrations_cache();
        let integrations = fetch_connected_integrations(&config).await;
        assert_eq!(integrations.len(), 2);
        // Sorted by toolkit name
        assert_eq!(integrations[0].toolkit, "gmail");
        assert_eq!(integrations[1].toolkit, "notion");
        assert_eq!(integrations[0].tools.len(), 1);
        assert_eq!(integrations[0].tools[0].name, "GMAIL_SEND_EMAIL");
    }

    #[tokio::test]
    async fn fetch_connected_integrations_via_mock_returns_empty_with_no_active() {
        let app = Router::new().route(
            "/agent-integrations/composio/connections",
            get(|| async {
                Json(json!({"success": true, "data": {"connections": [
                    {"id":"c1","toolkit":"gmail","status":"PENDING"}
                ]}}))
            }),
        );
        let base = start_mock_backend(app).await;
        let tmp = tempfile::tempdir().unwrap();
        let config = config_with_backend(&tmp, base);
        invalidate_connected_integrations_cache();
        let integrations = fetch_connected_integrations(&config).await;
        assert!(integrations.is_empty());
    }
}

// ── Helpers re-exported so callers can pull connection/tool types without
// reaching into the nested types module.
pub use super::types::{ComposioConnection as Connection, ComposioToolSchema as ToolSchemaType};
