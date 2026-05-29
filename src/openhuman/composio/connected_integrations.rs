use crate::openhuman::config::Config;
use crate::openhuman::context::prompt::{
    ConnectedIntegration, ConnectedIntegrationTool, GatedIntegrationTool,
};

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
    if age > CACHE_TTL {
        tracing::trace!(
            key = %key,
            age_ms = age.as_millis() as u64,
            ttl_ms = CACHE_TTL.as_millis() as u64,
            "[composio][integrations_cache] cached_active_integrations:expired"
        );
        return None;
    }
    tracing::trace!(
        key = %key,
        entries = cached.entries.len(),
        age_ms = age.as_millis() as u64,
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

    let mut slugs: Vec<&str> = integrations
        .iter()
        .filter(|i| i.connected)
        .map(|i| i.toolkit.as_str())
        .collect();
    slugs.sort();

    let mut hasher = DefaultHasher::new();
    slugs.hash(&mut hasher);
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
async fn fetch_connected_integrations_uncached(
    config: &Config,
) -> Option<Vec<ConnectedIntegration>> {
    use super::client::{create_composio_client, direct_list_connections, ComposioClientKind};
    use super::providers::toolkit_description;

    // Route via the mode-aware factory so the chat-agent's
    // "connected_integrations" view reflects the live tenant — backend
    // (tinyhumans) or direct (user's personal Composio). Prior to #1710
    // Wave 3 this path called `build_composio_client` directly, which
    // is backend-only — after a `composio.mode = "direct"` toggle the
    // cache kept replaying the tinyhumans-tenant connections back into
    // the integration overview (e.g. gmail / notion appearing as
    // connected in direct mode even when the user's direct tenant had
    // a different set of toolkits). Resolving per call closes the
    // loop: `ComposioConfigChangedSubscriber` invalidates the cache on
    // toggle and the next miss re-populates it from the live tenant.
    let kind = match create_composio_client(config) {
        Ok(kind) => kind,
        Err(e) => {
            tracing::debug!(
                error = %e,
                "[composio] fetch_connected_integrations: no client (not signed in?)"
            );
            return None;
        }
    };

    // Pull the allowlist + connections + tool catalogue. Backend mode
    // walks the tinyhumans tenant's curated allowlist via
    // `list_toolkits`; direct mode has no centralised allowlist (per
    // `ops::composio_list_toolkits`'s direct-mode branch) so the
    // user's set of active connections IS the universe of valid
    // toolkit arguments.
    //
    // On transient errors we return `None` instead of a degraded
    // `Some(Vec::new())` so `fetch_connected_integrations` does NOT
    // cache the failure. Caching an empty allowlist would hide every
    // integration from the orchestrator until the process restarts or
    // the cache is explicitly invalidated — a single 5xx during
    // startup would silently break delegation for the whole session.
    let (allowlisted_toolkits, connections, tools_by_toolkit): (
        Vec<String>,
        Vec<super::types::ComposioConnection>,
        Vec<super::types::ComposioToolSchema>,
    ) = match &kind {
        ComposioClientKind::Backend(client) => {
            let allowlist: Vec<String> = match client.list_toolkits().await {
                Ok(resp) => resp
                    .toolkits
                    .into_iter()
                    .map(|toolkit| toolkit.trim().to_ascii_lowercase())
                    .filter(|toolkit| !toolkit.is_empty())
                    .collect(),
                Err(e) => {
                    tracing::warn!(
                        "[composio] fetch_connected_integrations: list_toolkits (backend) failed: {e}"
                    );
                    return None;
                }
            };

            if allowlist.is_empty() {
                tracing::debug!(
                    "[composio] fetch_connected_integrations: backend allowlist is empty"
                );
                return Some(Vec::new());
            }

            let connections = match client.list_connections().await {
                Ok(resp) => resp.connections,
                Err(e) => {
                    tracing::warn!(
                        "[composio] fetch_connected_integrations: list_connections (backend) failed: {e}"
                    );
                    return None;
                }
            };

            // Tool catalogue scoped to the active subset only —
            // not-connected toolkits won't be invoked from a sub-agent.
            let connected_slugs_for_tools: Vec<String> = {
                let mut v: Vec<String> = connections
                    .iter()
                    .filter(|c| c.is_active())
                    .map(|c| c.normalized_toolkit())
                    .filter(|t| !t.is_empty())
                    .collect();
                v.sort();
                v.dedup();
                v
            };
            let tools = if connected_slugs_for_tools.is_empty() {
                Vec::new()
            } else {
                match client
                    .list_tools(Some(&connected_slugs_for_tools), None)
                    .await
                {
                    Ok(resp) => resp.tools,
                    Err(e) => {
                        tracing::warn!(
                            "[composio] fetch_connected_integrations: list_tools (backend) failed: {e}"
                        );
                        return None;
                    }
                }
            };

            (allowlist, connections, tools)
        }
        ComposioClientKind::Direct(direct) => {
            // Direct mode: walk the user's personal Composio tenant
            // for *connection state* (active accounts on their key) —
            // there's no central allowlist in direct mode, so the
            // active set IS the allowlist.
            //
            // Tool *schemas* are tenant-agnostic — Composio's action
            // definitions (e.g. GMAIL_SEND_EMAIL parameter shape) are
            // identical regardless of which Composio tenant a user is
            // connected via. So we best-effort fetch schemas through
            // the backend client (curated list_tools) if a backend
            // session is available, even though connection routing
            // goes to the user's direct tenant. This preserves the
            // chat agent's "21 gmail actions available" view while
            // execution itself (via `ComposioActionTool` / Wave 1
            // factory) still routes to the user's tenant. Direct-only
            // users without a backend session get empty tools — that
            // matches `composio_list_tools`'s direct-mode policy and
            // the `subagent_runner` LazyToolkitResolver still resolves
            // tools lazily at delegation time.
            let connections = match direct_list_connections(direct).await {
                Ok(resp) => resp.connections,
                Err(e) => {
                    tracing::warn!(
                        "[composio] fetch_connected_integrations: list_connections (direct) failed: {e:#}"
                    );
                    return None;
                }
            };
            let allowlist: Vec<String> = {
                let mut v: Vec<String> = connections
                    .iter()
                    .filter(|c| c.is_active())
                    .map(|c| c.normalized_toolkit())
                    .filter(|t| !t.is_empty())
                    .collect();
                v.sort();
                v.dedup();
                v
            };
            if allowlist.is_empty() {
                tracing::info!(
                    "[composio-direct] fetch_connected_integrations: direct tenant has no active connections; returning empty overview"
                );
                return Some(Vec::new());
            }
            tracing::debug!(
                connected = allowlist.len(),
                "[composio-direct] fetch_connected_integrations: using direct tenant's active set as allowlist (no central allowlist in direct mode)"
            );

            // Best-effort: pull tool schemas via the backend client
            // (definitional source). Failure is non-fatal — we fall
            // back to empty tools and let lazy resolution handle it.
            let tools = match super::client::build_composio_client(config) {
                Some(backend_client) => {
                    match backend_client.list_tools(Some(&allowlist), None).await {
                        Ok(resp) => {
                            tracing::debug!(
                            count = resp.tools.len(),
                            "[composio-direct] fetch_connected_integrations: pulled tool schemas from backend (tenant-agnostic definitional source)"
                        );
                            resp.tools
                        }
                        Err(e) => {
                            tracing::info!(
                            "[composio-direct] fetch_connected_integrations: backend list_tools failed (will use lazy fallback at delegation time): {e:#}"
                        );
                            Vec::new()
                        }
                    }
                }
                None => {
                    tracing::info!(
                        "[composio-direct] fetch_connected_integrations: no backend session for schema fetch; lazy fallback at delegation time"
                    );
                    Vec::new()
                }
            };
            (allowlist, connections, tools)
        }
    };

    // Active connection slugs (status filter mirrors the original logic).
    let connected_slugs: std::collections::HashSet<String> = connections
        .iter()
        .filter(|c| c.is_active())
        .map(|c| c.normalized_toolkit())
        .filter(|toolkit| !toolkit.is_empty())
        .collect();

    // Most-informative *non-active* status per toolkit slug. Lets the
    // integrations_agent spawn-gate (#2365) emit a precise message
    // when a connection row exists but isn't usable yet (`INITIATED`
    // — OAuth still in progress) or any longer (`EXPIRED` / `FAILED`)
    // — instead of the legacy generic "available but not authorized".
    //
    // Status priority (UI-actionability):
    //   1. EXPIRED  — reconnect path
    //   2. FAILED / ERROR — reconnect path
    //   3. INITIATED / INITIALIZING / PENDING — finish OAuth in browser
    //   4. anything else — passes through verbatim
    let non_active_status_by_slug: std::collections::HashMap<String, String> = {
        fn priority(status: &str) -> u8 {
            let s = status.trim().to_ascii_uppercase();
            match s.as_str() {
                "EXPIRED" => 4,
                "FAILED" | "ERROR" => 3,
                "INITIATED" | "INITIALIZING" | "PENDING" => 2,
                _ => 1,
            }
        }
        let mut map: std::collections::HashMap<String, (u8, String)> =
            std::collections::HashMap::new();
        for conn in connections.iter().filter(|c| !c.is_active()) {
            let slug = conn.normalized_toolkit();
            if slug.is_empty() {
                continue;
            }
            // Don't override an ACTIVE-slug — those carry no non-active
            // status from this map's perspective.
            if connected_slugs.contains(&slug) {
                continue;
            }
            let p = priority(&conn.status);
            map.entry(slug.clone())
                .and_modify(|cur| {
                    if p > cur.0 {
                        tracing::debug!(
                            target: "composio",
                            toolkit = %slug,
                            previous_status = %cur.1,
                            previous_priority = cur.0,
                            new_status = %conn.status,
                            new_priority = p,
                            "[composio] non_active_status_by_slug: upgraded most-informative status"
                        );
                        *cur = (p, conn.status.clone());
                    } else {
                        tracing::trace!(
                            target: "composio",
                            toolkit = %slug,
                            kept_status = %cur.1,
                            kept_priority = cur.0,
                            candidate_status = %conn.status,
                            candidate_priority = p,
                            "[composio] non_active_status_by_slug: kept higher-priority status"
                        );
                    }
                })
                .or_insert_with(|| {
                    tracing::debug!(
                        target: "composio",
                        toolkit = %slug,
                        status = %conn.status,
                        priority = p,
                        "[composio] non_active_status_by_slug: first non-active row"
                    );
                    (p, conn.status.clone())
                });
        }
        let final_map: std::collections::HashMap<String, String> =
            map.into_iter().map(|(k, (_, v))| (k, v)).collect();
        tracing::debug!(
            target: "composio",
            entries = final_map.len(),
            "[composio] non_active_status_by_slug: final map"
        );
        final_map
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
        let (tools, gated_tools): (Vec<ConnectedIntegrationTool>, Vec<GatedIntegrationTool>) =
            if connected {
                // Apply the same curated-whitelist + user-scope filter the
                // meta-tool layer uses, so the integrations_agent prompt
                // only advertises actions the agent is actually allowed to
                // call. One pref load per toolkit (not per action).
                //
                // Actions that the catalog *does* know about but the user's
                // current scope pref denies are routed into `gated_tools` so
                // the agent can honestly answer "I have this capability but
                // it needs the {scope} toggle in Connections → {toolkit}".
                // The agent cannot flip the scope itself — that's a UI-only
                // action. Without this gated surface the LLM has no way to
                // know the gated action exists at all and will tell the user
                // "I don't support that" — technically correct about its
                // callable surface, but misleading about the toolkit.
                let pref = super::providers::load_user_scope_or_default(slug).await;
                let mut visible: Vec<ConnectedIntegrationTool> = Vec::new();
                let mut gated: Vec<GatedIntegrationTool> = Vec::new();
                for t in tools_by_toolkit
                    .iter()
                    .filter(|t| t.function.name.starts_with(&action_prefix))
                {
                    if super::providers::is_action_visible_with_pref(&t.function.name, &pref) {
                        visible.push(ConnectedIntegrationTool {
                            name: t.function.name.clone(),
                            description: t.function.description.clone().unwrap_or_default(),
                            parameters: t.function.parameters.clone(),
                        });
                    } else if let Some(required_scope) =
                        super::providers::curated_scope_for(&t.function.name)
                    {
                        // Only surface CURATED actions as `gated` — uncurated
                        // tools (which fall through to `classify_unknown` and
                        // happen to land outside the user's pref) are not
                        // first-class user-facing capabilities, and listing
                        // them would clutter the prompt with internal slugs.
                        // Deliberately NO `parameters` field: the LLM should
                        // not be able to construct a call envelope; it can
                        // only describe + point at the unlock path.
                        // Ship the unlock path as data — single path today
                        // (the Connections UI toggle). The agent does NOT
                        // have a tool to flip scopes; that capability was
                        // removed because LLM-mediated scope elevation made
                        // the safety contract depend on model behavior and
                        // was a soft gate the model could route around. If
                        // more unlock paths exist in future (per-action
                        // approval modal, time-boxed elevation, etc.) they
                        // land here and the prompt renderer picks them up.
                        let scope_str = required_scope.as_str();
                        let unlock_paths = vec![format!(
                            "the user enables it themselves in \
                             Connections → {slug} → {scope_str}"
                        )];
                        gated.push(GatedIntegrationTool {
                            name: t.function.name.clone(),
                            description: t.function.description.clone().unwrap_or_default(),
                            required_scope: scope_str.to_string(),
                            unlock_paths,
                        });
                    }
                }
                tracing::debug!(
                    toolkit = %slug,
                    visible = visible.len(),
                    gated = gated.len(),
                    "[composio][scopes] integrations prompt action set"
                );
                (visible, gated)
            } else {
                (Vec::new(), Vec::new())
            };

        integrations.push(ConnectedIntegration {
            toolkit: slug.clone(),
            description: toolkit_description(slug).to_string(),
            tools,
            gated_tools,
            connected,
            non_active_status: if connected {
                None
            } else {
                non_active_status_by_slug.get(slug).cloned()
            },
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
            non_active_status = ?ci.non_active_status,
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
/// `tags` narrows the result by Composio action tag (OR semantics). Only
/// honoured for the GitHub toolkit; passed through to `list_tools` so the
/// backend can skip the repo-list force-include and return a focused set.
///
/// Returns an empty vec when the backend has no actions for the
/// toolkit (valid steady state for a freshly-authorised integration
/// whose catalogue hasn't been published yet). Returns `Err` only for
/// transport / auth failures the caller should surface to the user.
pub async fn fetch_toolkit_actions(
    client: &ComposioClient,
    toolkit: &str,
    tags: Option<&[String]>,
) -> anyhow::Result<Vec<ConnectedIntegrationTool>> {
    let toolkit_slug = toolkit.trim();
    if toolkit_slug.is_empty() {
        anyhow::bail!("fetch_toolkit_actions: toolkit must not be empty");
    }
    let effective_tags = if should_forward_tags(Some(&[toolkit_slug.to_string()])) {
        tags
    } else {
        None
    };
    tracing::debug!(toolkit = %toolkit_slug, ?effective_tags, "[composio] fetch_toolkit_actions");
    let resp = client
        .list_tools(Some(&[toolkit_slug.to_string()]), effective_tags)
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
