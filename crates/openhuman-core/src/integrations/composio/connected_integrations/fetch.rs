//! The public fetch surface for connected integrations:
//! [`fetch_connected_integrations`] / [`fetch_connected_integrations_status`]
//! (cache-fronted; a miss asks the module's `ListConnectedIntegrations`).

use std::sync::atomic::Ordering;

use crate::config::Config;
use crate::integrations::ConnectedIntegration;

use super::cache::{
    cache_generation, checked_cache_key, store_fetched_integrations, INTEGRATIONS_CACHE,
};

/// Fetch the user's active Composio connections and their available
/// tool actions, returning a prompt-ready summary.
///
/// This is the **single source of truth** for connected integration
/// data injected into system prompts — both the agent turn loop and
/// the debug dump CLI call this function.
///
/// Results are cached process-wide (keyed by storage scope and credential identity) and
/// returned instantly on subsequent calls. The cache is invalidated
/// when a connection changes (via [`invalidate_connected_integrations_cache`]
/// or `list_connections` reconciliation), or on process restart.
///
/// Best-effort: returns an empty vec when the user isn't signed in,
/// the backend is unreachable, or any step fails.
pub async fn fetch_connected_integrations(config: &Config) -> Vec<ConnectedIntegration> {
    match fetch_connected_integrations_status(config).await {
        FetchConnectedIntegrationsStatus::Authoritative(v) => v,
        FetchConnectedIntegrationsStatus::Unavailable => Vec::new(),
    }
}

/// Discriminated outcome from [`fetch_connected_integrations_status`].
///
/// Lets callers distinguish "the backend confirmed the user has zero
/// active connections right now" from "we couldn't talk to the backend
/// (no client, transient failure, …) and have no truth to report".
///
/// The legacy [`fetch_connected_integrations`] collapses both into an
/// empty `Vec`, which is fine for prompt-building (they look the same)
/// but dangerous for spawn-time allowlist gates — using empty as truth
/// in the unavailable case would silently wipe the user's allowlist
/// during a transient 5xx.
#[derive(Debug, Clone)]
pub enum FetchConnectedIntegrationsStatus {
    /// Backend was reachable. Vec may legitimately be empty (no
    /// allowlisted toolkits, or no active connections).
    Authoritative(Vec<ConnectedIntegration>),
    /// Backend wasn't reachable (no auth client, transient error). The
    /// caller should fall back to its prior snapshot rather than treat
    /// "no connections" as truth.
    Unavailable,
}

/// Status-returning variant of [`fetch_connected_integrations`].
///
/// Same caching, same cache-invalidation semantics — only the return
/// shape differs. Cache hits are by definition `Authoritative` because
/// we only cache the `Some(...)` arm of `_uncached` (i.e. results the
/// backend confirmed).
pub async fn fetch_connected_integrations_status(
    config: &Config,
) -> FetchConnectedIntegrationsStatus {
    // A task that lost its SaaS profile must neither reuse another profile's
    // snapshot nor request and cache connections under operator credentials.
    if let Err(error) = crate::storage::current_scope() {
        tracing::warn!(operation = "fetch", error_kind = ?error.kind(),
            "[composio][integrations] scope_resolution_refused");
        return FetchConnectedIntegrationsStatus::Unavailable;
    }
    // Composio switched off: nothing to ask. An authoritative empty set also
    // keeps the first turn from awaiting a hosted round trip.
    if config.composio.mode.trim() == crate::config::schema::COMPOSIO_MODE_DISABLED {
        return FetchConnectedIntegrationsStatus::Authoritative(Vec::new());
    }
    // The offline local token is a core identity, never a TinyHumans backend
    // credential. Asking the hosted integrations endpoint with it yields 401,
    // can race the scheduler gate into signed-out state, and cannot discover a
    // real connection. An authoritative empty set keeps this session local.
    // A stored API key outranks the local session (`resolve_backend_credential`),
    // so it still reaches the backend.
    let local_session_without_api_key = config.composio.mode.trim()
        != crate::config::schema::COMPOSIO_MODE_DIRECT
        && !crate::credentials::api_key::has_api_key(config).await
        && crate::credentials::session_state::get_session_token(config)
            .await
            .ok()
            .flatten()
            .is_some_and(|token| crate::credentials::session_state::is_local_session_token(&token));
    if local_session_without_api_key {
        return FetchConnectedIntegrationsStatus::Authoritative(Vec::new());
    }

    let Some(key) = checked_cache_key(config).await else {
        return FetchConnectedIntegrationsStatus::Unavailable;
    };
    // A connection event or a divergent list_connections response invalidates
    // this snapshot. Idle time alone must never put a network request on the
    // first-token path.
    if let Ok(guard) = INTEGRATIONS_CACHE.read() {
        if let Some(cached) = guard.get(&key) {
            let age = cached.cached_at.elapsed();
            tracing::debug!(
                count = cached.entries.len(),
                age_ms = age.as_millis() as u64,
                key = %key,
                "[composio][integrations] returning cached result"
            );
            return FetchConnectedIntegrationsStatus::Authoritative(cached.entries.clone());
        }
    }

    let epoch = cache_generation(&key);
    let generation = epoch.load(Ordering::SeqCst);
    match fetch_connected_integrations_uncached(config).await {
        Some(result) => {
            // Backend was reachable — cache the result (even if empty).
            if !store_fetched_integrations(key, &epoch, generation, result.clone()) {
                tracing::debug!("[composio][integrations] discarded fetch invalidated in flight");
                return FetchConnectedIntegrationsStatus::Unavailable;
            }
            FetchConnectedIntegrationsStatus::Authoritative(result)
        }
        None => {
            // No auth / client unavailable — do NOT cache so a
            // subsequent call with a different config can retry.
            FetchConnectedIntegrationsStatus::Unavailable
        }
    }
}

/// The cache-miss fetch: ask the module for the overview of the live route and
/// map it to the prompt types.
///
/// `None` when no route can be built (not signed in) or the module call fails,
/// so the caller does NOT cache the failure -- a single 5xx during startup must
/// not hide every integration for the rest of the session.
#[inline(never)]
fn fetch_connected_integrations_uncached(
    config: &Config,
) -> futures::future::BoxFuture<'_, Option<Vec<ConnectedIntegration>>> {
    Box::pin(async move {
        use tinyconnectors_bus::names::methods;
        use tinyconnectors_bus::{
            ComposioConnectedIntegrationsMode as Mode, ComposioConnectedIntegrationsResponse,
            ComposioListConnectedIntegrationsRequest,
        };

        let mode = match crate::integrations::composio::client::resolve_composio_route(config).await
        {
            Ok(crate::integrations::composio::client::ComposioRoute::Backend) => Mode::Backend,
            Ok(crate::integrations::composio::client::ComposioRoute::Direct(_)) => Mode::Direct,
            Err(error) => {
                tracing::debug!(%error,
                    "[composio][integrations] no client (not signed in?)");
                return None;
            }
        };
        match crate::integrations::composio::module_client::call::<
            _,
            ComposioConnectedIntegrationsResponse,
        >(
            config,
            methods::LIST_CONNECTED_INTEGRATIONS,
            ComposioListConnectedIntegrationsRequest { mode },
        )
        .await
        {
            Ok(resp) => Some(to_prompt_integrations(resp)),
            Err(error) => {
                tracing::warn!(?mode, %error,
                    "[composio][integrations] ListConnectedIntegrations failed");
                None
            }
        }
    })
}

/// Map the module's overview to the agent prompt's types. The unlock text for a
/// gated action is product copy, so it is built here from the scope.
pub(crate) fn to_prompt_integrations(
    resp: tinyconnectors_bus::ComposioConnectedIntegrationsResponse,
) -> Vec<ConnectedIntegration> {
    use crate::agent::prompts::{
        ConnectedIntegrationTool, GatedIntegrationTool, IntegrationConnection,
    };
    resp.integrations
        .into_iter()
        .map(|i| {
            let toolkit = i.toolkit;
            ConnectedIntegration {
                tools: i
                    .tools
                    .into_iter()
                    .map(|t| ConnectedIntegrationTool {
                        name: t.name,
                        description: t.description,
                        parameters: t.parameters,
                    })
                    .collect(),
                gated_tools: i
                    .gated_tools
                    .into_iter()
                    .map(|g| {
                        let scope = g.required_scope.as_str();
                        GatedIntegrationTool {
                            name: g.name,
                            description: g.description,
                            required_scope: scope.to_string(),
                            unlock_paths: vec![format!(
                                "the user enables it themselves in \
                                 Connections → {toolkit} → {scope}"
                            )],
                        }
                    })
                    .collect(),
                connections: i
                    .connections
                    .into_iter()
                    .map(|c| IntegrationConnection {
                        connection_id: c.connection_id,
                        label: c.label,
                        is_default: c.is_default,
                    })
                    .collect(),
                description: i.description,
                connected: i.connected,
                non_active_status: i.non_active_status,
                toolkit,
            }
        })
        .collect()
}

#[cfg(test)]
#[path = "fetch_tests.rs"]
mod tests;
