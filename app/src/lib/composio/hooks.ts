import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { canonicalizeComposioToolkitSlug } from '../../components/composio/toolkitMeta';
import { listConnections, listToolkits } from './composioApi';
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
  const [toolkits, setToolkits] = useState<string[]>([]);
  const [connections, setConnections] = useState<ComposioConnection[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const mountedRef = useRef(true);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const refresh = useCallback(async () => {
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
  }, []);

  // Initial fetch + polling.
  useEffect(() => {
    void refresh();
    if (pollIntervalMs <= 0) return;
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
  }, [refresh, pollIntervalMs]);

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
