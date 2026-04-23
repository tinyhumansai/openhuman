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
    // Reconcile the chat-runtime integrations cache against this fresh
    // snapshot. The desktop UI polls this RPC every 5 s, so any OAuth
    // completion that lands out-of-band from the event-bus invalidation
    // path (common on Windows when `wait_for_connection_active`'s 60 s
    // timeout fires before the user finishes the hosted flow) is still
    // reflected in chat within one poll interval.
    sync_cache_with_connections(&resp.connections);
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
    let toolkit = resolve_toolkit_for_connection(&client, connection_id)
        .await
        .ok();
    let resp = client
        .delete_connection(connection_id)
        .await
        .map_err(|e| format!("[composio] delete_connection failed: {e:#}"))?;
    if let Some(toolkit) = toolkit.as_deref() {
        let deleted =
            super::providers::profile::delete_connected_identity_facets(toolkit, connection_id);
        tracing::debug!(
            toolkit = %toolkit,
            connection_id = %connection_id,
            facets_deleted = deleted,
            "[composio] deleted connected identity facets after connection removal"
        );
    }
    crate::core::event_bus::publish_global(
        crate::core::event_bus::DomainEvent::ComposioConnectionDeleted {
            toolkit: toolkit.unwrap_or_else(|| "unknown".to_string()),
            connection_id: connection_id.to_string(),
        },
    );
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
    let facets = provider.identity_set(&profile);
    tracing::debug!(
        toolkit = %toolkit,
        facets_written = facets,
        "[composio] identity_set persisted profile facets from get_user_profile"
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
use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, RwLock};
use std::time::{Duration, Instant};

/// Defensive TTL on the integrations cache.
///
/// Background: the primary invalidation path is the
/// `ComposioConnectionCreated` → `wait_for_connection_active` bus flow
/// (see [`super::bus::ComposioConnectionCreatedSubscriber`]), which
/// polls the backend for up to 60 s after `composio_authorize` returns
/// a `connectUrl`. On Windows the OAuth round-trip can exceed that
/// window (Defender SmartScreen, slower browser launch, extra consent
/// dialogs), so the invalidation call never fires and the chat
/// runtime's cache stays frozen on the pre-connect snapshot even
/// though the Settings UI polls `composio_list_connections` every 5 s
/// and shows the user as "Connected".
///
/// The cross-platform defenses we layer on top:
///   1. [`composio_list_connections`] diff-invalidates the cache whenever
///      the backend's active-toolkit set diverges from what's cached,
///      so a running UI keeps the chat cache in sync within one poll
///      interval.
///   2. This TTL caps worst-case staleness at 60 s regardless of
///      whether the UI is open, the bus fires, or the user reconnected
///      out-of-band.
const CACHE_TTL: Duration = Duration::from_secs(60);

/// Cached entry: the integrations list plus the timestamp we wrote it.
#[derive(Clone)]
struct CachedIntegrations {
    entries: Vec<ConnectedIntegration>,
    cached_at: Instant,
}

/// Process-wide cache for connected integrations, keyed by the config
/// identity (the `config_path` string) so different user contexts don't
/// collide. Each entry is populated on first fetch and returned on
/// subsequent calls until explicitly invalidated or the TTL expires.
static INTEGRATIONS_CACHE: LazyLock<RwLock<HashMap<String, CachedIntegrations>>> =
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
/// new OAuth connection completes, by [`composio_list_connections`]
/// when it observes a divergence between the backend response and the
/// cached snapshot, and from tests. Clears the entire map because the
/// callers don't carry a config reference.
pub fn invalidate_connected_integrations_cache() {
    if let Ok(mut guard) = INTEGRATIONS_CACHE.write() {
        let entries = guard.len();
        guard.clear();
        tracing::info!(
            cached_keys = entries,
            "[composio][integrations] cache invalidated"
        );
    }
}

/// Collect the set of toolkit slugs marked `connected` in a snapshot.
///
/// Exposed to [`sync_cache_with_connections`] so it can diff the live
/// backend connection list against what the chat runtime currently
/// believes is connected.
fn connected_toolkit_set(integrations: &[ConnectedIntegration]) -> HashSet<String> {
    integrations
        .iter()
        .filter(|i| i.connected)
        .map(|i| i.toolkit.clone())
        .collect()
}

/// Reconcile the process-wide integrations cache with a fresh backend
/// `list_connections` response.
///
/// Called from [`composio_list_connections`], which the desktop UI
/// polls every 5 s (see `app/src/lib/composio/hooks.ts`). When the set
/// of ACTIVE/CONNECTED toolkits in the response differs from what's in
/// the cache, we invalidate so the chat runtime re-fetches on its next
/// `fetch_connected_integrations` call. This keeps tool availability
/// in chat in sync with the badge the user sees in Settings, even when
/// the primary event-bus invalidation path misses (e.g. Windows OAuth
/// flows that overrun the 60 s readiness poll).
fn sync_cache_with_connections(connections: &[super::types::ComposioConnection]) {
    let live_active: HashSet<String> = connections
        .iter()
        .filter(|c| matches!(c.status.as_str(), "ACTIVE" | "CONNECTED"))
        .map(|c| c.toolkit.clone())
        .collect();

    // Read once to decide whether any cache entry is out of sync. We
    // clone out the keys + connected sets so we can release the read
    // lock before taking the write lock.
    let divergent_keys: Vec<(String, HashSet<String>, HashSet<String>)> = {
        let Ok(guard) = INTEGRATIONS_CACHE.read() else {
            return;
        };
        guard
            .iter()
            .filter_map(|(key, cached)| {
                let cached_set = connected_toolkit_set(&cached.entries);
                if cached_set != live_active {
                    Some((key.clone(), cached_set, live_active.clone()))
                } else {
                    None
                }
            })
            .collect()
    };

    if divergent_keys.is_empty() {
        tracing::debug!(
            live_connected = live_active.len(),
            "[composio][integrations] list_connections matches cache — no invalidation needed"
        );
        return;
    }

    if let Ok(mut guard) = INTEGRATIONS_CACHE.write() {
        for (key, cached_set, live_set) in divergent_keys {
            // Diff logging — makes Windows-timing regressions easy to
            // catch in user-supplied debug dumps without leaking any
            // PII (toolkit slugs are public strings like "gmail").
            let added: Vec<&String> = live_set.difference(&cached_set).collect();
            let removed: Vec<&String> = cached_set.difference(&live_set).collect();
            tracing::info!(
                key = %key,
                ?added,
                ?removed,
                "[composio][integrations] cache diverges from backend — invalidating"
            );
            guard.remove(&key);
        }
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
/// (via [`invalidate_connected_integrations_cache`]), when a UI
/// `list_connections` poll observes a divergent live set, when
/// [`CACHE_TTL`] expires, or on process restart.
///
/// Best-effort: returns an empty vec when the user isn't signed in,
/// the backend is unreachable, or any step fails.
pub async fn fetch_connected_integrations(config: &Config) -> Vec<ConnectedIntegration> {
    let key = cache_key(config);

    // Fast path: return cached result if fresh. Stale entries fall
    // through to the backend fetch below so the chat runtime can never
    // be more than `CACHE_TTL` behind a real-world change.
    if let Ok(guard) = INTEGRATIONS_CACHE.read() {
        if let Some(cached) = guard.get(&key) {
            let age = cached.cached_at.elapsed();
            if age < CACHE_TTL {
                tracing::debug!(
                    count = cached.entries.len(),
                    age_ms = age.as_millis() as u64,
                    key = %key,
                    "[composio][integrations] returning cached result"
                );
                return cached.entries.clone();
            }
            tracing::info!(
                count = cached.entries.len(),
                age_ms = age.as_millis() as u64,
                ttl_ms = CACHE_TTL.as_millis() as u64,
                key = %key,
                "[composio][integrations] cache entry expired — refetching"
            );
        }
    }

    match fetch_connected_integrations_uncached(config).await {
        Some(result) => {
            // Backend was reachable — cache the result (even if empty).
            if let Ok(mut guard) = INTEGRATIONS_CACHE.write() {
                guard.insert(
                    key,
                    CachedIntegrations {
                        entries: result.clone(),
                        cached_at: Instant::now(),
                    },
                );
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
#[path = "ops_tests.rs"]
mod tests;

// ── Helpers re-exported so callers can pull connection/tool types without
// reaching into the nested types module.
pub use super::types::{ComposioConnection as Connection, ComposioToolSchema as ToolSchemaType};
