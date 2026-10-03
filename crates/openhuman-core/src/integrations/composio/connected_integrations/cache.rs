//! The process-wide connected-integrations cache: [`CachedIntegrations`],
//! [`INTEGRATIONS_CACHE`], its readers/invalidation
//! ([`cached_active_integrations`], [`cached_active_integrations_including_expired`],
//! [`invalidate_connected_integrations_cache`]), and the reconciliation
//! helpers ([`connected_set_hash`], [`sync_cache_with_connections`]) that
//! keep it in sync with a fresh backend `list_connections` response.

use crate::agent::prompts::ConnectedIntegration;
use crate::config::Config;
use sha2::{Digest, Sha256};

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, RwLock};
use std::time::Instant;

// ── Prompt integration discovery ────────────────────────────────────

/// Cached entry: the integrations list plus the timestamp we wrote it.
#[derive(Clone)]
pub(crate) struct CachedIntegrations {
    pub(crate) entries: Vec<ConnectedIntegration>,
    pub(crate) cached_at: Instant,
}

/// Process-wide cache for connected integrations, keyed by the Composio
/// credential identity ([`cache_key`]) so two agents on different
/// credentials never read each other's connections.
pub(crate) static INTEGRATIONS_CACHE: LazyLock<RwLock<HashMap<String, CachedIntegrations>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// Prevent an in-flight startup warm from restoring a snapshot that was
/// invalidated while its backend requests were running.
pub(crate) static CACHE_GENERATION: AtomicU64 = AtomicU64::new(0);

/// Crate-wide test serialization lock for all tests that mutate or read
/// the process-global `INTEGRATIONS_CACHE`. Defined here so it is shared
/// by every `cfg(test)` module in this crate (ops_tests, tools_tests, …).
/// Poison-recovery (`unwrap_or_else`) keeps a panicking test from
/// permanently blocking later ones.
#[cfg(test)]
pub(crate) fn composio_cache_test_lock() -> tokio::sync::MutexGuard<'static, ()> {
    COMPOSIO_CACHE_TEST_LOCK.blocking_lock()
}

/// Async form for `#[tokio::test]` bodies, so the guard may be held across `.await`.
#[cfg(test)]
pub(crate) async fn composio_cache_test_lock_async() -> tokio::sync::MutexGuard<'static, ()> {
    COMPOSIO_CACHE_TEST_LOCK.lock().await
}

#[cfg(test)]
static COMPOSIO_CACHE_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Bind a cached integration list to its backend endpoint, backend credential,
/// and effective Composio credential identity. Digests keep secrets out of
/// cache keys and logs while preventing cross-agent cache hits after rotation.
pub(crate) fn cache_key(config: &Config) -> String {
    let backend_url = crate::backend::base_url(&config.api_url)
        .map(|url| crate::util::url::normalize_backend_api_base_url(&url))
        .unwrap_or_default();
    cache_key_with_backend_url(config, &backend_url)
}

fn cache_key_with_backend_url(config: &Config, backend_url: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(b"openhuman-integrations-cache-v3\0");
    digest.update(config.config_path.to_string_lossy().as_bytes());
    digest.update(b"\0");
    digest.update(backend_url.as_bytes());
    digest.update(b"\0");
    match crate::security::credentials::session_support::resolve_backend_credential(config) {
        Ok(credential) => {
            digest.update(if credential.is_api_key() {
                b"api-key\0"
            } else {
                b"session\0"
            });
            digest.update(credential.secret().as_bytes());
        }
        Err(_) => digest.update(b"unavailable"),
    }
    digest.update(b"\0composio\0");
    let composio = &config.composio;
    digest.update(composio.mode.trim().as_bytes());
    digest.update(b"\0");
    digest.update(composio.entity_id.trim().as_bytes());
    digest.update(b"\0");
    if let Some(host) = composio.host_credential.as_ref() {
        digest.update(host.api_key().as_bytes());
        digest.update(b"\0");
        digest.update(host.entity().as_bytes());
        if let Some(urls) = host.direct_base_urls() {
            digest.update(urls.v2.as_bytes());
            digest.update(b"\0");
            digest.update(urls.v3.as_bytes());
        }
    } else {
        let inline_key = composio
            .api_key
            .as_deref()
            .map(str::trim)
            .filter(|key| !key.is_empty());
        if composio.mode.trim() == crate::config::schema::COMPOSIO_MODE_DIRECT {
            if let Ok(Some(stored)) = crate::security::credentials::get_composio_api_key(config) {
                digest.update(stored.as_bytes());
            } else if let Some(key) = inline_key {
                digest.update(key.as_bytes());
            }
        } else if let Some(key) = inline_key {
            digest.update(key.as_bytes());
        }
    }
    format!("composio:{}", hex::encode(digest.finalize()))
}

/// Clear cached connected integrations so the next call to
/// [`fetch_connected_integrations`] hits the backend again.
///
/// Called by [`crate::integrations::composio::bus::ComposioConnectionCreatedSubscriber`] when a
/// new OAuth connection completes, by [`composio_list_connections`]
/// when it observes a divergence between the backend response and the
/// cached snapshot, and from tests. Clears the entire map because the
/// callers don't carry a config reference.
pub fn invalidate_connected_integrations_cache() {
    if let Ok(mut guard) = INTEGRATIONS_CACHE.write() {
        let entries = guard.len();
        guard.clear();
        CACHE_GENERATION.fetch_add(1, Ordering::SeqCst);
        tracing::info!(
            cached_keys = entries,
            "[composio][integrations] cache invalidated"
        );
    }
}

/// Read-only snapshot of the currently cached connected integrations for
/// the given config, or [`None`] when the cache is empty or
/// the lock is held by a writer.
///
/// Designed for hot-path callers that want a cheap "what does the cache
/// already say?" probe without triggering a backend fetch. Connection
/// changes invalidate the snapshot and trigger a background refresh.
///
/// `try_read` (not `read`) so a writer in progress — e.g. the UI poll
/// repopulating the cache — never blocks a turn. Worst case the agent
/// sees `None` for one turn while the writer holds the lock; the next
/// turn picks up the value naturally.
///
/// Freshness is driven by connection events and the Settings connection-list
/// reconciliation, never by an idle-time expiry on the chat critical path.
pub fn cached_active_integrations(config: &Config) -> Option<Vec<ConnectedIntegration>> {
    read_cached_integrations(config)
}

/// Compatibility alias for callers preserving the last-known snapshot when a
/// connection-change refresh fails. Entries have no time-based expiry.
pub fn cached_active_integrations_including_expired(
    config: &Config,
) -> Option<Vec<ConnectedIntegration>> {
    read_cached_integrations(config)
}

fn read_cached_integrations(config: &Config) -> Option<Vec<ConnectedIntegration>> {
    read_cached_integrations_from(&INTEGRATIONS_CACHE, config)
}

/// [`read_cached_integrations`] over an explicit map, so the credential-keyed
/// lookup can be exercised against a private map instead of the process-wide
/// [`INTEGRATIONS_CACHE`], which unrelated code paths clear at any time (see
/// [`invalidate_connected_integrations_cache`]).
pub(crate) fn read_cached_integrations_from(
    cache: &RwLock<HashMap<String, CachedIntegrations>>,
    config: &Config,
) -> Option<Vec<ConnectedIntegration>> {
    let key = cache_key(config);
    let guard = match cache.try_read() {
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
    tracing::trace!(
        key = %key,
        entries = cached.entries.len(),
        age_ms = age.as_millis() as u64,
        "[composio][integrations_cache] cached_active_integrations:hit"
    );
    Some(cached.entries.clone())
}

#[cfg(test)]
#[path = "cache_tests.rs"]
mod tests;

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
/// stripped by [`crate::tools::orchestrator_tools::collect_orchestrator_tools`]
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
/// flows that overrun the 60 s readiness poll). Returns whether an entry was
/// invalidated so the caller can eagerly re-warm it away from a chat turn.
pub(crate) fn sync_cache_with_connections(
    connections: &[crate::integrations::composio::types::ComposioConnection],
) -> bool {
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
            return false;
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
        return false;
    }

    if let Ok(mut guard) = INTEGRATIONS_CACHE.write() {
        CACHE_GENERATION.fetch_add(1, Ordering::SeqCst);
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
        return true;
    }
    false
}
