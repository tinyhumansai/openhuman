import { invoke } from '@tauri-apps/api/core';
import { useCallback } from 'react';

import { useAppDispatch, useAppSelector } from '../store/hooks';
import {
  setDownloadTriggered,
  setModelError,
  setModelLoading,
  setModelStatus,
  type ModelStatus,
} from '../store/modelSlice';

/**
 * Hook to read model status from Redux and provide control actions.
 * Status polling and auto-download are handled by ModelProvider.
 */
export const useModelStatus = () => {
  const dispatch = useAppDispatch();
  const model = useAppSelector(state => state.model);

  const fetchStatus = useCallback(async () => {
    try {
      const result = await invoke<ModelStatus>('model_get_status');
      dispatch(setModelStatus(result));
      return result;
    } catch (error) {
      console.error('[useModelStatus] Failed to fetch status:', error);
      dispatch(setModelError(error instanceof Error ? error.message : 'Failed to fetch status'));
      return null;
    }
  }, [dispatch]);

  const startDownload = useCallback(async () => {
    try {
      dispatch(setModelLoading(true));
      dispatch(setModelError(null));
      dispatch(setDownloadTriggered(true));
      await invoke('model_start_download');
      await fetchStatus();
    } catch (error) {
      console.error('[useModelStatus] Failed to start download:', error);
      dispatch(
        setModelError(error instanceof Error ? error.message : 'Failed to download model')
      );
    }
  }, [dispatch, fetchStatus]);

  const ensureLoaded = useCallback(async () => {
    try {
      dispatch(setModelLoading(true));
      dispatch(setModelError(null));
      await invoke('model_ensure_loaded');
      await fetchStatus();
    } catch (error) {
      console.error('[useModelStatus] Failed to load model:', error);
      dispatch(setModelError(error instanceof Error ? error.message : 'Failed to load model'));
    }
  }, [dispatch, fetchStatus]);

  const unload = useCallback(async () => {
    try {
      await invoke('model_unload');
      await fetchStatus();
    } catch (error) {
      console.error('[useModelStatus] Failed to unload model:', error);
    }
  }, [fetchStatus]);

  return {
    status: model,
    isAvailable: model.available,
    isLoaded: model.loaded,
    isLoading: model.loading,
    isDownloaded: model.downloaded,
    downloadProgress: model.downloadProgress,
    error: model.error,
    startDownload,
    ensureLoaded,
    unload,
    refresh: fetchStatus,
  };
};

export type { ModelStatus };
