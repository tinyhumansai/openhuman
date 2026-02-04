import { invoke } from '@tauri-apps/api/core';
import { useCallback, useEffect, useState } from 'react';

/**
 * Model status from Rust backend
 */
export interface ModelStatus {
  available: boolean;
  loaded: boolean;
  loading: boolean;
  downloadProgress: number | null;
  error: string | null;
  modelPath: string | null;
}

const DEFAULT_STATUS: ModelStatus = {
  available: false,
  loaded: false,
  loading: false,
  downloadProgress: null,
  error: null,
  modelPath: null,
};

/**
 * Hook to monitor and control local AI model status
 */
export const useModelStatus = (pollInterval = 1000) => {
  const [status, setStatus] = useState<ModelStatus>(DEFAULT_STATUS);
  const [isPolling, setIsPolling] = useState(false);

  // Fetch current status from backend
  const fetchStatus = useCallback(async () => {
    try {
      const result = await invoke<ModelStatus>('model_get_status');
      setStatus(result);
      return result;
    } catch (error) {
      console.error('[useModelStatus] Failed to fetch status:', error);
      setStatus(prev => ({
        ...prev,
        error: error instanceof Error ? error.message : 'Failed to fetch status',
      }));
      return null;
    }
  }, []);

  // Check if model API is available
  const checkAvailability = useCallback(async () => {
    try {
      const available = await invoke<boolean>('model_is_available');
      setStatus(prev => ({ ...prev, available }));
      return available;
    } catch (error) {
      console.error('[useModelStatus] Failed to check availability:', error);
      return false;
    }
  }, []);

  // Start loading/downloading the model
  const ensureLoaded = useCallback(async () => {
    try {
      setStatus(prev => ({ ...prev, loading: true, error: null }));
      setIsPolling(true);
      await invoke('model_ensure_loaded');
      await fetchStatus();
      setIsPolling(false);
    } catch (error) {
      console.error('[useModelStatus] Failed to load model:', error);
      setStatus(prev => ({
        ...prev,
        loading: false,
        error: error instanceof Error ? error.message : 'Failed to load model',
      }));
      setIsPolling(false);
    }
  }, [fetchStatus]);

  // Unload the model from memory
  const unload = useCallback(async () => {
    try {
      await invoke('model_unload');
      await fetchStatus();
    } catch (error) {
      console.error('[useModelStatus] Failed to unload model:', error);
    }
  }, [fetchStatus]);

  // Initial check and polling setup
  useEffect(() => {
    // Initial fetch
    fetchStatus();

    // Check availability
    checkAvailability();
  }, [fetchStatus, checkAvailability]);

  // Polling when loading/downloading
  useEffect(() => {
    if (!isPolling && !status.loading) return;

    const interval = setInterval(async () => {
      const newStatus = await fetchStatus();
      // Stop polling when loading is done
      if (newStatus && !newStatus.loading) {
        setIsPolling(false);
      }
    }, pollInterval);

    return () => clearInterval(interval);
  }, [isPolling, status.loading, pollInterval, fetchStatus]);

  return {
    status,
    isAvailable: status.available,
    isLoaded: status.loaded,
    isLoading: status.loading,
    downloadProgress: status.downloadProgress,
    error: status.error,
    ensureLoaded,
    unload,
    refresh: fetchStatus,
  };
};
