import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { isLocalSessionToken } from '../../utils/localSession';
import { openhumanComposioGetMode } from '../../utils/tauriCommands';
import { getCoreStateSnapshot } from '../coreState/store';
import { getToolkitCatalog, invalidateToolkitCatalogCache } from './catalogCache';
import { COMPOSIO_FETCH_TIMEOUT_MS, listAgentReadyToolkits, listConnections } from './composioApi';
import { clearConnectionCache, readConnectionCache, writeConnectionCache } from './connectionCache';
import { canonicalizeComposioToolkitSlug } from './toolkitSlug';
import type { ComposioConnection, ComposioToolkitCatalogEntry } from './types';

// ── cold-start retry ──────────────────────────────────────────────

/**
 * Extra silent attempts before a failed Connections fetch surfaces the
 * stale-status banner (#4290). The Composio RPC is slow on cold start, so the
 * first ~8s budget is often blown but a retry seconds later succeeds — exactly
 * what the user achieves manually with "Try again". One retry (2 attempts
 * total) self-heals the common case while keeping the worst-case skeleton
 * window bounded (~2×8s + backoff) on a genuine outage.
 */
const COMPOSIO_FETCH_RETRY_ATTEMPTS = 1;
/** Backoff between a failed attempt and the silent retry. */
const COMPOSIO_FETCH_RETRY_BACKOFF_MS = 400;

const delay = (ms: number): Promise<void> => new Promise(resolve => setTimeout(resolve, ms));

type LegResult<T> = { value: T } | { error: string };

/**
 * Run `fn` with bounded silent retries. Resolves to `{ value }` on the first
 * success or `{ error }` after the final attempt fails — it never rejects, so
 * both Connections legs can settle independently. Aborts between attempts when
 * the hook has unmounted so we don't sleep/setState into a dead component.
 */
async function fetchLegWithRetries<T>(
  label: string,
  fn: () => Promise<T>,
  isMounted: () => boolean
): Promise<LegResult<T>> {
  let lastError = '';
  for (let attempt = 0; attempt <= COMPOSIO_FETCH_RETRY_ATTEMPTS; attempt += 1) {
    try {
      return { value: await fn() };
    } catch (err) {
      lastError = err instanceof Error ? err.message : String(err);
      const willRetry = attempt < COMPOSIO_FETCH_RETRY_ATTEMPTS && isMounted();
      console.warn(
        `[composio] refresh leg=${label} attempt=${attempt + 1} failed: ${lastError}${
          willRetry ? ' — retrying silently' : ''
        }`
      );
      if (!willRetry) break;
      await delay(COMPOSIO_FETCH_RETRY_BACKOFF_MS);
      if (!isMounted()) break;
    }
  }
  return { error: lastError };
}

// ── useComposioIntegrations ───────────────────────────────────────

export interface UseComposioIntegrationsResult {
  /** Toolkit slugs enabled on the backend allowlist. */
  toolkits: string[];
  /**
   * Live Composio catalog entries (dynamic name/logo/description/
   * categories) keyed by canonical lowercased slug. Empty when the
   * core/backend predates the dynamic catalog — consumers then fall
   * back to the local `toolkitMeta` derivation.
   */
  catalogByToolkit: Map<string, ComposioToolkitCatalogEntry>;
  /** Best (highest-status) connection keyed by lowercased toolkit slug. */
  connectionByToolkit: Map<string, ComposioConnection>;
  /** All connections keyed by lowercased toolkit slug, sorted by status (ACTIVE first, then by createdAt). */
  connectionsByToolkit: Map<string, ComposioConnection[]>;
  /** Whether the initial fetch is still in flight. */
  loading: boolean;
  /** Last error message from either fetch, if any. */
  error: string | null;
  /** Force a refetch of toolkits + connections. */
  refresh: () => Promise<void>;
}

/**
 * Fetches the Composio toolkit allowlist and current connections.
 *
 * Composio is always enabled on the core side — it's proxied through
 * our backend, uses the same JWT as every other core RPC call, and has
 * no client-side feature toggle. So the only failure modes here are
 * network/backend errors, which get surfaced via `error`.
 *
 * On mount we do one request of each, then re-fetch connections on a
 * `pollIntervalMs` loop so the UI reacts to OAuth completions without
 * the user having to manually refresh. Toolkits are only refetched on
 * explicit `refresh()` because the allowlist is stable.
 */
export function useComposioIntegrations(pollIntervalMs = 5_000): UseComposioIntegrationsResult {
  const isLocalSession = isLocalSessionToken(getCoreStateSnapshot().snapshot.sessionToken);
  // Seed from the durable connection cache so a cold restart re-paints the
  // last-known connected toolkits instantly instead of flashing an empty
  // loading skeleton (#4273, AC1). The live fetch below still runs on mount and
  // reconciles within a few seconds, so a stale seed is self-correcting.
  const [toolkits, setToolkits] = useState<string[]>(() => readConnectionCache()?.toolkits ?? []);
  const [catalog, setCatalog] = useState<ComposioToolkitCatalogEntry[]>(
    () => readConnectionCache()?.catalog ?? []
  );
  const [connections, setConnections] = useState<ComposioConnection[]>(
    () => readConnectionCache()?.connections ?? []
  );
  // No skeleton when we already have a cached snapshot to show.
  const [loading, setLoading] = useState(() => readConnectionCache() == null);
  const [error, setError] = useState<string | null>(null);
  const [fetchEnabled, setFetchEnabled] = useState<boolean | null>(() =>
    isLocalSession ? null : true
  );
  const mountedRef = useRef(true);
  // Bumped whenever the Composio client identity changes (backend ↔ direct /
  // BYO key) via the config-changed handler. Any refresh/poll started under an
  // older generation must not commit its result — otherwise an in-flight fetch
  // can repopulate the cache the handler just cleared with the previous
  // tenant's connections, painting phantom activations on the next restart
  // (PR #4288).
  const configGenerationRef = useRef(0);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const resolveFetchEnabled = useCallback(async (): Promise<boolean> => {
    if (!isLocalSession) {
      if (mountedRef.current) setFetchEnabled(true);
      return true;
    }
    try {
      const res = await openhumanComposioGetMode();
      const enabled = Boolean(res.result?.api_key_set);
      if (mountedRef.current) setFetchEnabled(enabled);
      return enabled;
    } catch (err) {
      console.warn(
        '[composio] failed to resolve direct-mode api key status:',
        err instanceof Error ? err.message : String(err)
      );
      if (mountedRef.current) setFetchEnabled(false);
      return false;
    }
  }, [isLocalSession]);

  const refresh = useCallback(async () => {
    const generation = configGenerationRef.current;
    const enabled = fetchEnabled ?? (await resolveFetchEnabled());
    if (!enabled) {
      // Direct mode with no API key configured: there can be no connections, so
      // drop any cached snapshot too — otherwise a removed key would still
      // re-paint phantom "connected" toolkits on the next restart.
      clearConnectionCache();
      if (mountedRef.current) {
        setToolkits([]);
        setCatalog([]);
        setConnections([]);
        setError(null);
        setLoading(false);
      }
      return;
    }

    let nextError: string | null = null;
    try {
      // Bound both fetches so the loading skeleton can't pin past ~8s per
      // attempt on a cold cache / down backend (COMPOSIO_FETCH_TIMEOUT_MS),
      // and retry each leg silently before surfacing the stale banner (#4290)
      // so a single slow cold-start RPC doesn't look broken. `loading` stays
      // true across the retry window, so the page shows the skeleton — never
      // the error banner — until a leg has genuinely exhausted its attempts.
      const isMounted = () => mountedRef.current;
      const [toolkitsResult, connectionsResult] = await Promise.all([
        fetchLegWithRetries(
          'toolkits',
          () => getToolkitCatalog({ timeoutMs: COMPOSIO_FETCH_TIMEOUT_MS }),
          isMounted
        ),
        fetchLegWithRetries(
          'connections',
          () => listConnections({ timeoutMs: COMPOSIO_FETCH_TIMEOUT_MS }),
          isMounted
        ),
      ]);
      // Drop results from a client that has since been swapped out — committing
      // them would revive the previous tenant's state post-invalidation.
      if (!mountedRef.current || generation !== configGenerationRef.current) return;

      if ('value' in toolkitsResult) {
        setToolkits(toolkitsResult.value.toolkits ?? []);
        setCatalog(toolkitsResult.value.catalog ?? []);
      } else {
        nextError = toolkitsResult.error;
      }

      if ('value' in connectionsResult) {
        const freshConnections = connectionsResult.value.connections ?? [];
        setConnections(freshConnections);
        // Persist the latest activation snapshot for instant cold-start paint.
        // Pair it with this round's toolkit data when that fetch also
        // succeeded; otherwise leave the cached toolkit/catalog fields intact
        // (writeConnectionCache merges, so `undefined` keeps the prior value).
        writeConnectionCache({
          connections: freshConnections,
          toolkits: 'value' in toolkitsResult ? (toolkitsResult.value.toolkits ?? []) : undefined,
          catalog: 'value' in toolkitsResult ? (toolkitsResult.value.catalog ?? []) : undefined,
        });
      } else if (!nextError) {
        // fetchLegWithRetries already logged each failed attempt for this leg.
        nextError = connectionsResult.error;
      }

      setError(nextError);
    } finally {
      if (mountedRef.current) setLoading(false);
    }
  }, [fetchEnabled, resolveFetchEnabled]);

  // Initial fetch + polling.
  useEffect(() => {
    void refresh();
    if (pollIntervalMs <= 0 || fetchEnabled !== true) return;
    const id = window.setInterval(() => {
      const generation = configGenerationRef.current;
      void listConnections({ timeoutMs: COMPOSIO_FETCH_TIMEOUT_MS })
        .then(resp => {
          if (!mountedRef.current || generation !== configGenerationRef.current) return;
          const freshConnections = resp.connections ?? [];
          setConnections(freshConnections);
          // Keep the durable cache current with each poll so a restart paints
          // the freshest activation state (merge preserves toolkit/catalog).
          writeConnectionCache({ connections: freshConnections });
        })
        .catch(err => {
          console.warn(
            '[composio] polling connections failed:',
            err instanceof Error ? err.message : String(err)
          );
        });
    }, pollIntervalMs);
    return () => window.clearInterval(id);
  }, [refresh, pollIntervalMs, fetchEnabled]);

  // [composio-cache] Listen for a window-level "config changed" event
  // emitted by ComposioPanel when the user flips backend ↔ direct or
  // stores/clears the BYO API key. Without this, the integrations panel
  // keeps showing the previous tenant's connections for up to one poll
  // interval (5s) — visible enough to look like a bug (#1710). On the
  // event we trigger a full refresh which re-fetches toolkits +
  // connections against the new client. We also rely on the Rust-side
  // ComposioConfigChanged bus event to invalidate the core-side cache;
  // the window event is purely an in-renderer signal.
  useEffect(() => {
    const onConfigChanged = () => {
      console.debug('[composio-cache] window:composio:config-changed → refresh()');
      // Invalidate any refresh/poll already in flight under the previous client
      // so it can't write its result back after the caches below are cleared.
      configGenerationRef.current += 1;
      // The Composio client identity changed (backend ↔ direct / BYO key),
      // so the cached catalog AND connections belong to the previous tenant.
      // Drop both before refetching, mirroring the core-side
      // ComposioConfigChanged eviction.
      invalidateToolkitCatalogCache();
      clearConnectionCache();
      if (isLocalSession) {
        void resolveFetchEnabled().then(enabled => {
          if (enabled) {
            void refresh();
            return;
          }
          if (mountedRef.current) {
            setToolkits([]);
            setCatalog([]);
            setConnections([]);
            setError(null);
            setLoading(false);
          }
        });
        return;
      }
      void refresh();
    };
    window.addEventListener('composio:config-changed', onConfigChanged);
    return () => window.removeEventListener('composio:config-changed', onConfigChanged);
  }, [isLocalSession, refresh, resolveFetchEnabled]);

  const score = (status: string): number => {
    const s = status.toUpperCase();
    if (s === 'ACTIVE' || s === 'CONNECTED') return 3;
    if (s === 'PENDING' || s === 'INITIATED' || s === 'INITIALIZING') return 2;
    if (s === 'FAILED' || s === 'ERROR' || s === 'EXPIRED') return 1;
    return 0;
  };

  const catalogByToolkit = useMemo(() => {
    const map = new Map<string, ComposioToolkitCatalogEntry>();
    for (const entry of catalog) {
      map.set(canonicalizeComposioToolkitSlug(entry.slug), entry);
    }
    return map;
  }, [catalog]);

  const connectionByToolkit = useMemo(() => {
    const map = new Map<string, ComposioConnection>();
    for (const conn of connections) {
      const key = canonicalizeComposioToolkitSlug(conn.toolkit);
      const existing = map.get(key);
      if (!existing || score(conn.status) > score(existing.status)) {
        map.set(key, conn);
      }
    }
    return map;
  }, [connections]);

  const connectionsByToolkit = useMemo(() => {
    const map = new Map<string, ComposioConnection[]>();
    for (const conn of connections) {
      const key = canonicalizeComposioToolkitSlug(conn.toolkit);
      const existing = map.get(key) ?? [];
      existing.push(conn);
      map.set(key, existing);
    }
    for (const [key, conns] of map) {
      conns.sort((a, b) => {
        const diff = score(b.status) - score(a.status);
        if (diff !== 0) return diff;
        return (a.createdAt ?? '').localeCompare(b.createdAt ?? '');
      });
      map.set(key, conns);
    }
    return map;
  }, [connections]);

  return {
    toolkits,
    catalogByToolkit,
    connectionByToolkit,
    connectionsByToolkit,
    loading,
    error,
    refresh,
  };
}

// ── useAgentReadyComposioToolkits ─────────────────────────────────

export interface UseAgentReadyComposioToolkitsResult {
  /** Lowercased slugs of toolkits that ship an agent-ready catalog. */
  agentReady: ReadonlySet<string>;
  /** Whether the initial fetch is still in flight. */
  loading: boolean;
  /** Last error message from the fetch, if any. */
  error: string | null;
}

/**
 * Fetches the set of Composio toolkits that have an agent-ready
 * curated catalog on the core side. The list changes only with
 * core releases, so we fetch once on mount and never refresh.
 *
 * Used by the Skills grid (issue #2283) to flag connected
 * toolkits without a catalog as "preview / coming soon" so users
 * don't trigger the max-iterations failure that an uncurated
 * connection causes when the agent calls `composio_list_tools`.
 *
 * On fetch failure we return an empty set and surface the error
 * — the UI degrades to "no preview labels" rather than
 * incorrectly labelling everything as preview.
 */
export function useAgentReadyComposioToolkits(): UseAgentReadyComposioToolkitsResult {
  const [agentReady, setAgentReady] = useState<ReadonlySet<string>>(() => new Set());
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const mountedRef = useRef(true);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  useEffect(() => {
    listAgentReadyToolkits()
      .then(resp => {
        if (!mountedRef.current) return;
        const normalized = (resp.toolkits ?? []).map(canonicalizeComposioToolkitSlug);
        setAgentReady(new Set(normalized));
        setError(null);
      })
      .catch(err => {
        if (!mountedRef.current) return;
        const message = err instanceof Error ? err.message : String(err);
        console.warn('[composio] agent-ready toolkits fetch failed:', message);
        setError(message);
      })
      .finally(() => {
        if (mountedRef.current) setLoading(false);
      });
  }, []);

  return { agentReady, loading, error };
}
