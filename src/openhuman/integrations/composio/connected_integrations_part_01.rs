use crate::openhuman::agent::context::prompt::{
    ConnectedIntegration, ConnectedIntegrationTool, GatedIntegrationTool,
};
use crate::openhuman::config::Config;

use super::client::ComposioClient;
use super::ops::should_forward_tags;

use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, RwLock};
use std::time::{Duration, Instant};

// ── Prompt integration discovery ────────────────────────────────────

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
pub(crate) const CACHE_TTL: Duration = Duration::from_secs(60);

/// Cached entry: the integrations list plus the timestamp we wrote it.
#[derive(Clone)]
pub(crate) struct CachedIntegrations {
    pub(crate) entries: Vec<ConnectedIntegration>,
    pub(crate) cached_at: Instant,
}

/// Process-wide cache for connected integrations, keyed by the config
/// identity (the `config_path` string) so different user contexts don't
/// collide. Each entry is populated on first fetch and returned on
/// subsequent calls until explicitly invalidated or the TTL expires.
pub(crate) static INTEGRATIONS_CACHE: LazyLock<RwLock<HashMap<String, CachedIntegrations>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// Crate-wide test serialization lock for all tests that mutate or read
/// the process-global `INTEGRATIONS_CACHE`. Defined here so it is shared
/// by every `cfg(test)` module in this crate (ops_tests, tools_tests, …).
/// Poison-recovery (`unwrap_or_else`) keeps a panicking test from
/// permanently blocking later ones.
#[cfg(test)]
pub(crate) fn composio_cache_test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// Derive a stable cache key from a [`Config`]. We use the stringified
/// `config_path` because it uniquely identifies a user context (it
/// resolves to the per-user openhuman dir).
pub(crate) fn cache_key(config: &Config) -> String {
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

/// Read-only snapshot of the currently cached connected integrations for
/// the given config, or [`None`] when the cache is empty, expired, or
/// the lock is held by a writer.
///
/// Designed for hot-path callers that want a cheap "what does the cache
/// already say?" probe without triggering a backend fetch. The agent
/// harness uses this on every turn to detect mid-session connection
/// changes — it relies on the desktop UI's 5 s `composio_list_connections`
/// poll (which calls into [`fetch_connected_integrations`] and
/// repopulates this cache) plus the event-driven invalidation path to
/// keep the cache current.
///
/// `try_read` (not `read`) so a writer in progress — e.g. the UI poll
/// repopulating the cache — never blocks a turn. Worst case the agent
/// sees `None` for one turn while the writer holds the lock; the next
/// turn picks up the value naturally.
///
/// TTL is enforced defensively: entries older than [`CACHE_TTL`] are
/// treated as missing even though they're still in the map (a stale
/// entry would otherwise pin the agent to a frozen view if every
/// invalidation path silently failed).
pub fn cached_active_integrations(config: &Config) -> Option<Vec<ConnectedIntegration>> {
    read_cached_integrations(config, false)
}

/// Like [`cached_active_integrations`] but returns the last cached snapshot
/// even when it has aged past [`CACHE_TTL`].
///
/// Intended ONLY as a transient-failure fallback: when a live fetch reports
/// `Unavailable`/times out, preserving the last-known integrations is strictly
/// better than collapsing the delegation surface to an empty set (which drops
/// `delegate_to_integrations_agent` and silently disables channel tool-calling).
/// Without this, a backend blip that lands just after the 60 s TTL expiry still
/// wipes tool-calling despite having a perfectly good previous snapshot. A fresh
/// fetch repopulates the cache the moment the backend recovers, so the stale
/// window is bounded by the outage, not by this call.
pub fn cached_active_integrations_including_expired(
    config: &Config,
) -> Option<Vec<ConnectedIntegration>> {
    read_cached_integrations(config, true)
}

/// Shared reader for the integrations cache. `allow_expired` bypasses the
/// [`CACHE_TTL`] freshness check for the transient-failure fallback path.
fn read_cached_integrations(
    config: &Config,
    allow_expired: bool,
) -> Option<Vec<ConnectedIntegration>> {
    let key = cache_key(config);
    let guard = match INTEGRATIONS_CACHE.try_read() {
        Ok(g) => g,
        Err(_) => {
            tracing::trace!(
                key = %key,
                "[composio][integrations_cache] cached_active_integrations:lock_contended"
            );
            return None;
        }
    };
    let Some(cached) = guard.get(&key) else {
        tracing::trace!(
            key = %key,
            "[composio][integrations_cache] cached_active_integrations:miss"
        );
        return None;
    };
    let age = cached.cached_at.elapsed();
    if !allow_expired && age > CACHE_TTL {
        tracing::trace!(
            key = %key,
            age_ms = age.as_millis() as u64,
            ttl_ms = CACHE_TTL.as_millis() as u64,
            "[composio][integrations_cache] cached_active_integrations:expired"
        );
        return None;
    }
    // Surface a *very* stale fallback so an unusually long backend outage is
    // observable rather than silently pinning the agent to an ancient snapshot.
    if allow_expired && age > 5 * CACHE_TTL {
        tracing::warn!(
            key = %key,
            age_ms = age.as_millis() as u64,
            ttl_ms = CACHE_TTL.as_millis() as u64,
            "[composio][integrations_cache] serving a heavily-stale integrations snapshot on transient-failure fallback (backend outage?)"
        );
    }
    tracing::trace!(
        key = %key,
        entries = cached.entries.len(),
        age_ms = age.as_millis() as u64,
        allow_expired,
        "[composio][integrations_cache] cached_active_integrations:hit"
    );
    Some(cached.entries.clone())
}

/// Stable hash of the *routing-relevant* slice of a connected-integrations
/// snapshot.
///
/// Two snapshots produce the same hash iff they would synthesise the
/// same `delegate_<toolkit>` tool set in the orchestrator's
/// function-calling schema. The hash is:
///
///   - **Order-independent** — callers don't need to sort the input.
///   - **Description-insensitive** — Composio catalogue text edits don't
///     trigger a refresh. The schema's tool-description field still
///     picks up new text on the next *real* (membership-changing)
///     refresh, so descriptions are never permanently stale.
///   - **Process-local** — [`std::collections::hash_map::DefaultHasher`]
///     is randomly seeded per process. Fine because we only compare
///     hashes within one process lifetime.
///
/// Only `connected == true` entries contribute. Unconnected toolkits are
/// stripped by [`super::super::tools::orchestrator_tools::collect_orchestrator_tools`]
/// anyway, so churn among the unconnected set never changes the agent's
/// surface and shouldn't trigger a refresh.
pub fn connected_set_hash(integrations: &[ConnectedIntegration]) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut pairs: Vec<(&str, Vec<&str>)> = integrations
        .iter()
        .filter(|i| i.connected)
        .map(|i| {
            let mut ids: Vec<&str> = i
                .connections
                .iter()
                .map(|c| c.connection_id.as_str())
                .collect();
            ids.sort();
            (i.toolkit.as_str(), ids)
        })
        .collect();
    pairs.sort_by(|a, b| a.0.cmp(b.0));

    let mut hasher = DefaultHasher::new();
    pairs.hash(&mut hasher);
    hasher.finish()
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
pub(crate) fn sync_cache_with_connections(connections: &[super::types::ComposioConnection]) {
    let live_active: HashSet<String> = connections
        .iter()
        .filter(|c| c.is_active())
        .map(|c| c.normalized_toolkit())
        .filter(|toolkit| !toolkit.is_empty())
        .collect();

    // Collect active connection IDs per toolkit to detect multi-account changes
    let live_ids: std::collections::HashMap<String, Vec<String>> = {
        let mut ids: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        for c in connections.iter().filter(|c| c.is_active()) {
            let tk = c.normalized_toolkit();
            if !tk.is_empty() {
                ids.entry(tk).or_default().push(c.id.clone());
            }
        }
        for v in ids.values_mut() {
            v.sort();
        }
        ids
    };

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
                // Also check per-toolkit connection IDs (not just counts)
                let ids_match = cached.entries.iter().all(|i| {
                    let mut cached_ids: Vec<&str> = i
                        .connections
                        .iter()
                        .map(|c| c.connection_id.as_str())
                        .collect();
                    cached_ids.sort();
                    let empty = Vec::new();
                    let live = live_ids.get(&i.toolkit).unwrap_or(&empty);
                    cached_ids.len() == live.len()
                        && cached_ids
                            .iter()
                            .zip(live.iter())
                            .all(|(a, b)| *a == b.as_str())
                });
                if cached_set != live_active || !ids_match {
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
                return FetchConnectedIntegrationsStatus::Authoritative(cached.entries.clone());
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
            FetchConnectedIntegrationsStatus::Authoritative(result)
        }
        None => {
            // No auth / client unavailable — do NOT cache so a
            // subsequent call with a different config can retry.
            FetchConnectedIntegrationsStatus::Unavailable
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
fn connectable_toolkit_slugs(
    toolkits: &[String],
    catalog: &[super::types::ComposioToolkitCatalogEntry],
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
fn resolve_toolkit_description(
    catalog_descriptions: &std::collections::HashMap<String, String>,
    slug: &str,
) -> String {
    catalog_descriptions
        .get(slug)
        .cloned()
        .unwrap_or_else(|| super::providers::toolkit_description(slug).to_string())
}
