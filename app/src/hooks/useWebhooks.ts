import debug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';

import { useCoreState } from '../providers/CoreStateProvider';
import { tunnelsApi } from '../services/api/tunnelsApi';
import { getCoreHttpBaseUrl } from '../services/coreRpcClient';
import { useAppDispatch, useAppSelector } from '../store/hooks';
import {
  addActivity,
  addTunnel,
  removeTunnel,
  setError,
  setLoading,
  setRegistrations,
  setTunnels,
  type WebhookActivityEntry,
} from '../store/webhooksSlice';
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
  const dispatch = useAppDispatch();
  const { tunnels, registrations, activity, loading, error } = useAppSelector(
    state => state.webhooks
  );
  const token = snapshot.sessionToken;
  const [coreConnected, setCoreConnected] = useState(false);
  const eventSourceRef = useRef<EventSource | null>(null);

  // ── Load registrations + logs from core RPC ──────────────────────────────
  const loadCoreData = useCallback(async () => {
    try {
      const [regsResponse, logsResponse] = await Promise.all([
        openhumanWebhooksListRegistrations(),
        openhumanWebhooksListLogs(100),
      ]);
      dispatch(setRegistrations(regsResponse.result.result.registrations));

      // Seed activity from debug logs
      const logs = logsResponse.result.result.logs;
      for (const entry of logs.reverse()) {
        dispatch(addActivity(logToActivity(entry)));
      }
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
  }, [dispatch]);

  // ── Fetch tunnels from backend API ───────────────────────────────────────
  const fetchTunnels = useCallback(async () => {
    dispatch(setLoading(true));
    try {
      const data = await tunnelsApi.getTunnels();
      dispatch(setTunnels(data));
      log('Fetched %d tunnels', data.length);
    } catch (err) {
      const msg = err instanceof Error ? err.message : 'Failed to fetch tunnels';
      dispatch(setError(msg));
      log('Error fetching tunnels: %s', msg);
    }
  }, [dispatch]);

  // ── Subscribe to SSE for real-time webhook events ────────────────────────
  useEffect(() => {
    let cancelled = false;

    const connect = async () => {
      try {
        const baseUrl = await getCoreHttpBaseUrl();
        if (cancelled) return;

        const es = new EventSource(`${baseUrl}/events/webhooks`);
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
  }, [loadCoreData]);

  // ── Initial data load ────────────────────────────────────────────────────
  useEffect(() => {
    if (!token) return;
    void fetchTunnels();
    void loadCoreData();
  }, [token, fetchTunnels, loadCoreData]);

  // ── CRUD actions ─────────────────────────────────────────────────────────
  const createTunnel = useCallback(
    async (name: string, description?: string) => {
      try {
        const tunnel = await tunnelsApi.createTunnel({ name, description });
        dispatch(addTunnel(tunnel));
        log('Created tunnel: %s (%s)', tunnel.name, tunnel.uuid);
        return tunnel;
      } catch (err) {
        const msg = err instanceof Error ? err.message : 'Failed to create tunnel';
        dispatch(setError(msg));
        throw err;
      }
    },
    [dispatch]
  );

  const deleteTunnel = useCallback(
    async (id: string) => {
      try {
        await tunnelsApi.deleteTunnel(id);
        dispatch(removeTunnel(id));
        log('Deleted tunnel: %s', id);
      } catch (err) {
        const msg = err instanceof Error ? err.message : 'Failed to delete tunnel';
        dispatch(setError(msg));
        throw err;
      }
    },
    [dispatch]
  );

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
        dispatch(setRegistrations(response.result.result.registrations));
        log('Registered echo for tunnel %s', tunnelUuid);
      } catch (err) {
        const msg = err instanceof Error ? err.message : 'Failed to register echo';
        dispatch(setError(msg));
        throw err;
      }
    },
    [dispatch]
  );

  const unregisterEcho = useCallback(
    async (tunnelUuid: string) => {
      try {
        const response = await openhumanWebhooksUnregisterEcho(tunnelUuid);
        dispatch(setRegistrations(response.result.result.registrations));
        log('Unregistered echo for tunnel %s', tunnelUuid);
      } catch (err) {
        const msg = err instanceof Error ? err.message : 'Failed to unregister echo';
        dispatch(setError(msg));
        throw err;
      }
    },
    [dispatch]
  );

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
