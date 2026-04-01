import debug from 'debug';
import { useCallback, useEffect } from 'react';

import { tunnelsApi } from '../services/api/tunnelsApi';
import { useAppDispatch, useAppSelector } from '../store/hooks';
import { addTunnel, removeTunnel, setError, setLoading, setTunnels } from '../store/webhooksSlice';

const log = debug('webhooks');

/**
 * Hook for managing webhook tunnels.
 * Fetches tunnels from the backend API and provides CRUD operations.
 */
export function useWebhooks() {
  const dispatch = useAppDispatch();
  const { tunnels, registrations, activity, loading, error } = useAppSelector(
    state => state.webhooks
  );
  const token = useAppSelector(state => state.auth.token);

  // Fetch tunnels on mount (when authenticated)
  useEffect(() => {
    if (!token) return;

    const fetchTunnels = async () => {
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
    };

    fetchTunnels();
  }, [token, dispatch]);

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
    dispatch(setLoading(true));
    try {
      const data = await tunnelsApi.getTunnels();
      dispatch(setTunnels(data));
    } catch (err) {
      const msg = err instanceof Error ? err.message : 'Failed to refresh tunnels';
      dispatch(setError(msg));
    }
  }, [dispatch]);

  return {
    tunnels,
    registrations,
    activity,
    loading,
    error,
    createTunnel,
    deleteTunnel,
    refreshTunnels,
  };
}
