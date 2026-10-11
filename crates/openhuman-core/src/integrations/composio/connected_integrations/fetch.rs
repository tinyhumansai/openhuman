//! The public fetch surface for connected integrations:
//! [`fetch_connected_integrations`] / [`fetch_connected_integrations_status`]
//! (cache-fronted, calling [`super::fetch_uncached::fetch_connected_integrations_uncached`]
//! on a miss), plus their small toolkit-membership/description helpers.

use std::sync::atomic::Ordering;
use std::time::Instant;

use crate::agent::prompts::ConnectedIntegration;
use crate::config::Config;

use super::cache::{cache_key, CachedIntegrations, CACHE_GENERATION, INTEGRATIONS_CACHE};
use super::fetch_uncached::fetch_connected_integrations_uncached;

/// Fetch the user's active Composio connections and their available
/// tool actions, returning a prompt-ready summary.
///
/// This is the **single source of truth** for connected integration
/// data injected into system prompts — both the agent turn loop and
/// the debug dump CLI call this function.
///
/// Results are cached process-wide (keyed by credential identity) and
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
    if config.composio.mode.trim() != crate::config::schema::COMPOSIO_MODE_DIRECT
        && !crate::security::credentials::api_key::has_api_key(config)
        && crate::security::credentials::session_support::get_session_token(config)
            .ok()
            .flatten()
            .is_some_and(|token| {
                crate::security::credentials::session_support::is_local_session_token(&token)
            })
    {
        return FetchConnectedIntegrationsStatus::Authoritative(Vec::new());
    }
    let key = cache_key(config);

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

    let generation = CACHE_GENERATION.load(Ordering::SeqCst);
    match fetch_connected_integrations_uncached(config).await {
        Some(result) => {
            // Backend was reachable — cache the result (even if empty).
            if let Ok(mut guard) = INTEGRATIONS_CACHE.write() {
                if CACHE_GENERATION.load(Ordering::SeqCst) == generation {
                    guard.insert(
                        key,
                        CachedIntegrations {
                            entries: result.clone(),
                            cached_at: Instant::now(),
                        },
                    );
                } else {
                    tracing::debug!(
                        "[composio][integrations] discarded fetch invalidated in flight"
                    );
                    return FetchConnectedIntegrationsStatus::Unavailable;
                }
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

/// The connectable toolkit slugs to surface in the agent prompt, aligned
/// with the backend's execution gate.
///
/// Prefers the dynamic catalog's **enabled** entries (openhuman PR #3933 /
/// backend #1012). The backend gate (`isToolkitConnectable` →
/// `getProjectList().filter(p => p.enabled)`) and `catalog[].enabled` are
/// driven by the same project auth-config status, so sourcing membership from
/// `catalog.filter(enabled)` pins the prompt's advertised set to exactly what
/// connect/authorize/execute will actually allow — it can't drift even if the
/// backend later changes how the flat `toolkits` array is projected.
///
/// Falls back to the back-compat `toolkits` array when the catalog is absent
/// (backends predating the dynamic catalog send only `toolkits`); without the
/// fallback, membership against an old core would collapse to empty and the
/// agent would lose every integration. Disabled catalog entries are dropped
/// because the gate would reject them — advertising them would only invite
/// failed delegations. Slugs are trimmed + lowercased to match downstream
/// canonicalisation.
pub(crate) fn connectable_toolkit_slugs(
    toolkits: &[String],
    catalog: &[crate::integrations::composio::types::ComposioToolkitCatalogEntry],
) -> Vec<String> {
    let normalize = |s: &str| s.trim().to_ascii_lowercase();
    if catalog.is_empty() {
        toolkits
            .iter()
            .map(|t| normalize(t))
            .filter(|t| !t.is_empty())
            .collect()
    } else {
        catalog
            .iter()
            .filter(|entry| entry.enabled.unwrap_or(false))
            .map(|entry| normalize(&entry.slug))
            .filter(|slug| !slug.is_empty())
            .collect()
    }
}

/// Choose the one-line description rendered for a toolkit in the agent
/// prompt's `## Connected Integrations` block.
///
/// Prefers the backend's **dynamic catalog** description (openhuman PR #3933
/// / backend #1012 — `GET /agent-integrations/composio/toolkits` now returns
/// a `catalog[]` with per-toolkit metadata) so the orchestrator advertises
/// what Composio actually offers. Falls back to the hardcoded
/// `toolkit_description` table when the catalog omits the toolkit or ships an
/// empty description — i.e. older backends that predate the dynamic catalog,
/// or project toolkits whose Composio metadata join produced no blurb. Keyed
/// by lowercased slug to match the canonicalised allowlist.
pub(crate) fn resolve_toolkit_description(
    catalog_descriptions: &std::collections::HashMap<String, String>,
    slug: &str,
) -> String {
    catalog_descriptions.get(slug).cloned().unwrap_or_else(|| {
        crate::integrations::composio::contract::toolkit_description(slug).to_string()
    })
}
