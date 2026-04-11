import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

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
  /** True when composio is disabled on the core side. */
  disabled: boolean;
}

/**
 * Fetches the Composio toolkit allowlist and current connections.
 *
 * On mount it does one request of each, then re-fetches connections on
 * a `pollIntervalMs` loop so the UI reacts to OAuth completions without
 * the user having to manually refresh. Toolkits are only refetched on
 * explicit `refresh()` because the allowlist is stable.
 *
 * When the core reports that composio is disabled (feature toggle off,
 * or integrations.enabled=false) the hook short-circuits into
 * `disabled=true` and stops polling — callers can use that to show an
 * "integrations disabled" hint instead of a spinner.
 */
export function useComposioIntegrations(pollIntervalMs = 5_000): UseComposioIntegrationsResult {
  const [toolkits, setToolkits] = useState<string[]>([]);
  const [connections, setConnections] = useState<ComposioConnection[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [disabled, setDisabled] = useState(false);
  const mountedRef = useRef(true);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const refresh = useCallback(async () => {
    try {
      const [toolkitsResp, connectionsResp] = await Promise.all([
        listToolkits(),
        listConnections(),
      ]);
      if (!mountedRef.current) return;
      setToolkits(toolkitsResp.toolkits ?? []);
      setConnections(connectionsResp.connections ?? []);
      setDisabled(false);
      setError(null);
    } catch (err) {
      if (!mountedRef.current) return;
      const message = err instanceof Error ? err.message : String(err);
      // Detect the "composio disabled" error the Rust ops layer emits
      // so we can render a distinct state rather than a red error.
      if (/composio is disabled/i.test(message)) {
        setDisabled(true);
        setError(null);
      } else {
        setError(message);
      }
    } finally {
      if (mountedRef.current) setLoading(false);
    }
  }, []);

  // Initial fetch + polling.
  useEffect(() => {
    void refresh();
    if (pollIntervalMs <= 0) return;
    const id = window.setInterval(() => {
      if (disabled) return;
      void listConnections()
        .then(resp => {
          if (!mountedRef.current) return;
          setConnections(resp.connections ?? []);
        })
        .catch(() => {
          /* swallow — non-fatal for poll cadence */
        });
    }, pollIntervalMs);
    return () => window.clearInterval(id);
  }, [refresh, pollIntervalMs, disabled]);

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
      const key = conn.toolkit.toLowerCase();
      const existing = map.get(key);
      if (!existing || score(conn.status) > score(existing.status)) {
        map.set(key, conn);
      }
    }
    return map;
  }, [connections]);

  return { toolkits, connectionByToolkit, loading, error, refresh, disabled };
}
