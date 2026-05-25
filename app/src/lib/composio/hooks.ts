import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { isLocalSessionToken } from '../../utils/localSession';
import { openhumanComposioGetMode } from '../../utils/tauriCommands';
import { getCoreStateSnapshot } from '../coreState/store';
import { listAgentReadyToolkits, listConnections, listToolkits } from './composioApi';
import { canonicalizeComposioToolkitSlug } from './toolkitSlug';
import type { ComposioConnection } from './types';

// ── useComposioIntegrations ───────────────────────────────────────

export interface UseComposioIntegrationsResult {
  /** Toolkit slugs enabled on the backend allowlist. */
  toolkits: string[];
  /** Connections keyed by lowercased toolkit slug. */
  connectionByToolkit: Map<string, ComposioConnection>;
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
  const [toolkits, setToolkits] = useState<string[]>([]);
  const [connections, setConnections] = useState<ComposioConnection[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [fetchEnabled, setFetchEnabled] = useState<boolean | null>(() =>
    isLocalSession ? null : true
  );
  const mountedRef = useRef(true);

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
    const enabled = fetchEnabled ?? (await resolveFetchEnabled());
    if (!enabled) {
      if (mountedRef.current) {
        setToolkits([]);
        setConnections([]);
        setError(null);
        setLoading(false);
      }
      return;
    }

    let nextError: string | null = null;
    try {
      const [toolkitsResult, connectionsResult] = await Promise.allSettled([
        listToolkits(),
        listConnections(),
      ]);
      if (!mountedRef.current) return;

      if (toolkitsResult.status === 'fulfilled') {
        setToolkits(toolkitsResult.value.toolkits ?? []);
      } else {
        const message =
          toolkitsResult.reason instanceof Error
            ? toolkitsResult.reason.message
            : String(toolkitsResult.reason);
        console.warn('[composio] toolkit fetch failed:', message);
        nextError = message;
      }

      if (connectionsResult.status === 'fulfilled') {
        setConnections(connectionsResult.value.connections ?? []);
      } else {
        const message =
          connectionsResult.reason instanceof Error
            ? connectionsResult.reason.message
            : String(connectionsResult.reason);
        console.warn('[composio] connection fetch failed:', message);
        if (!nextError) nextError = message;
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
      void listConnections()
        .then(resp => {
          if (!mountedRef.current) return;
          setConnections(resp.connections ?? []);
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
      if (isLocalSession) {
        void resolveFetchEnabled().then(enabled => {
          if (enabled) {
            void refresh();
            return;
          }
          if (mountedRef.current) {
            setToolkits([]);
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

  const connectionByToolkit = useMemo(() => {
    const map = new Map<string, ComposioConnection>();
    // Preference order: ACTIVE/CONNECTED > PENDING > anything else.
    const score = (status: string): number => {
      const s = status.toUpperCase();
      if (s === 'ACTIVE' || s === 'CONNECTED') return 3;
      if (s === 'PENDING' || s === 'INITIATED' || s === 'INITIALIZING') return 2;
      if (s === 'FAILED' || s === 'ERROR' || s === 'EXPIRED') return 1;
      return 0;
    };
    for (const conn of connections) {
      const key = canonicalizeComposioToolkitSlug(conn.toolkit);
      const existing = map.get(key);
      if (!existing || score(conn.status) > score(existing.status)) {
        map.set(key, conn);
      }
    }
    return map;
  }, [connections]);

  return { toolkits, connectionByToolkit, loading, error, refresh };
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
