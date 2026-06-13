import debug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';

import type { Tunnel, TunnelRegistration, WebhookActivityEntry } from '../features/webhooks/types';
import { useCoreState } from '../providers/CoreStateProvider';
import { tunnelsApi } from '../services/api/tunnelsApi';
import {
  buildWebhookEventsUrl,
  getCoreHttpBaseUrl,
  getCoreRpcToken,
  subscribeCoreRpcTokenInvalidated,
} from '../services/coreRpcClient';
import {
  openhumanWebhooksListLogs,
  openhumanWebhooksListRegistrations,
  openhumanWebhooksRegisterEcho,
  openhumanWebhooksUnregisterEcho,
  type WebhookDebugLogEntry,
} from '../utils/tauriCommands';

const log = debug('webhooks');

/** Convert a debug log entry to an activity entry for the ring buffer. */
function logToActivity(entry: WebhookDebugLogEntry): WebhookActivityEntry {
  return {
    correlation_id: entry.correlation_id,
    tunnel_name: entry.tunnel_name,
    method: entry.method,
    path: entry.path,
    status_code: entry.status_code,
    skill_id: entry.skill_id,
    timestamp: entry.updated_at || entry.timestamp,
  };
}

/**
 * Hook for managing webhook tunnels, registrations, and live activity.
 *
 * - Fetches tunnels from the backend API (CRUD)
 * - Fetches registrations + debug logs from the Rust core (via JSON-RPC)
 * - Subscribes to SSE /events/webhooks for real-time activity updates
 */
export function useWebhooks() {
  const { snapshot } = useCoreState();
  const token = snapshot.sessionToken;
  const [tunnels, setTunnels] = useState<Tunnel[]>([]);
  const [registrations, setRegistrations] = useState<TunnelRegistration[]>([]);
  const [activity, setActivity] = useState<WebhookActivityEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [coreConnected, setCoreConnected] = useState(false);
  const [coreRpcToken, setCoreRpcToken] = useState<string | null>(null);
  const eventSourceRef = useRef<EventSource | null>(null);

  // ── Load registrations + logs from core RPC ──────────────────────────────
  const loadCoreData = useCallback(async () => {
    try {
      const [regsResponse, logsResponse] = await Promise.all([
        openhumanWebhooksListRegistrations(),
        openhumanWebhooksListLogs(100),
      ]);
      setRegistrations(regsResponse.result.result.registrations);

      // Seed activity from debug logs
      const logs = logsResponse.result.result.logs;
      setActivity(logs.reverse().map(logToActivity));
      log(
        'Loaded %d registrations, %d logs from core',
        regsResponse.result.result.registrations.length,
        logs.length
      );
    } catch (err) {
      log(
        'Core RPC not available (registrations/logs): %s',
        err instanceof Error ? err.message : err
      );
    }
  }, []);

  // ── Fetch tunnels from backend API ───────────────────────────────────────
  const fetchTunnels = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const data = await tunnelsApi.getTunnels();
      setTunnels(data);
      log('Fetched %d tunnels', data.length);
    } catch (err) {
      const msg = err instanceof Error ? err.message : 'Failed to fetch tunnels';
      setError(msg);
      log('Error fetching tunnels: %s', msg);
    } finally {
      setLoading(false);
    }
  }, []);

  // Resolve the core RPC bearer used to authenticate the SSE stream. We
  // re-resolve when:
  //   - the session token flips (login / logout / cloud-mode switch), OR
  //   - the core RPC token cache is invalidated (core restart per #1922 —
  //     the new core mints a fresh `OPENHUMAN_CORE_TOKEN`, so the old SSE
  //     stream is now authenticated against a dead bearer and must be
  //     torn down).
  useEffect(() => {
    let cancelled = false;
    const resolve = () => {
      void getCoreRpcToken()
        .then(resolved => {
          if (!cancelled) setCoreRpcToken(resolved ?? null);
        })
        .catch(() => {
          if (!cancelled) setCoreRpcToken(null);
        });
    };
    resolve();
    const unsubscribe = subscribeCoreRpcTokenInvalidated(resolve);
    return () => {
      cancelled = true;
      unsubscribe();
    };
  }, [token]);

  // ── Subscribe to SSE for real-time webhook events ────────────────────────
  // The token is part of the dep array so a rotation tears down the current
  // EventSource and opens a fresh one with the new credential. Without a
  // token the stream is unauthenticated, so we skip rather than open a
  // request that the server will reject and EventSource will reconnect to
  // forever.
  useEffect(() => {
    let cancelled = false;

    if (!coreRpcToken) {
      log('SSE skip: no core RPC token available yet');
      return () => {
        cancelled = true;
      };
    }

    const connect = async () => {
      try {
        const baseUrl = await getCoreHttpBaseUrl();
        if (cancelled) return;

        const url = buildWebhookEventsUrl(baseUrl, coreRpcToken);
        if (!url) {
          log('SSE skip: buildWebhookEventsUrl returned null');
          return;
        }
        const es = new EventSource(url);
        eventSourceRef.current = es;

        es.addEventListener('webhooks_debug', () => {
          setCoreConnected(true);
          // Reload registrations + logs on any debug event (registration change, new log, etc.)
          void loadCoreData();
        });

        es.onopen = () => {
          setCoreConnected(true);
          log('SSE connected to /events/webhooks');
        };

        es.onerror = () => {
          setCoreConnected(false);
        };
      } catch {
        setCoreConnected(false);
      }
    };

    void connect();

    return () => {
      cancelled = true;
      if (eventSourceRef.current) {
        eventSourceRef.current.close();
        eventSourceRef.current = null;
      }
      setCoreConnected(false);
    };
  }, [coreRpcToken, loadCoreData]);

  // ── Initial data load ────────────────────────────────────────────────────
  useEffect(() => {
    if (!token) return;
    void fetchTunnels();
    void loadCoreData();
  }, [token, fetchTunnels, loadCoreData]);

  // ── CRUD actions ─────────────────────────────────────────────────────────
  const createTunnel = useCallback(async (name: string, description?: string) => {
    const tunnel = await tunnelsApi.createTunnel({ name, description });
    setTunnels(current => [...current, tunnel]);
    log('Created tunnel: %s (%s)', tunnel.name, tunnel.uuid);
    return tunnel;
  }, []);

  const deleteTunnel = useCallback(async (id: string) => {
    await tunnelsApi.deleteTunnel(id);
    setTunnels(current => current.filter(tunnel => tunnel.id !== id));
    log('Deleted tunnel: %s', id);
  }, []);

  const refreshTunnels = useCallback(async () => {
    await fetchTunnels();
    await loadCoreData();
  }, [fetchTunnels, loadCoreData]);

  // ── Echo registration ────────────────────────────────────────────────────
  const registerEcho = useCallback(
    async (tunnelUuid: string, tunnelName?: string, backendTunnelId?: string) => {
      try {
        const response = await openhumanWebhooksRegisterEcho(
          tunnelUuid,
          tunnelName,
          backendTunnelId
        );
        setRegistrations(response.result.result.registrations);
        log('Registered echo for tunnel %s', tunnelUuid);
      } catch (err) {
        const msg = err instanceof Error ? err.message : 'Failed to register echo';
        setError(msg);
        throw err;
      }
    },
    []
  );

  const unregisterEcho = useCallback(async (tunnelUuid: string) => {
    try {
      const response = await openhumanWebhooksUnregisterEcho(tunnelUuid);
      setRegistrations(response.result.result.registrations);
      log('Unregistered echo for tunnel %s', tunnelUuid);
    } catch (err) {
      const msg = err instanceof Error ? err.message : 'Failed to unregister echo';
      setError(msg);
      throw err;
    }
  }, []);

  return {
    tunnels,
    registrations,
    activity,
    loading,
    error,
    coreConnected,
    createTunnel,
    deleteTunnel,
    refreshTunnels,
    registerEcho,
    unregisterEcho,
  };
}
