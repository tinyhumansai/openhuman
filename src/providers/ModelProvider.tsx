import { invoke } from '@tauri-apps/api/core';
import { platform } from '@tauri-apps/plugin-os';
import { useEffect } from 'react';

import { useAppDispatch, useAppSelector } from '../store/hooks';
import {
  type ModelStatus,
  setDownloadTriggered,
  setModelError,
  setModelLoading,
  setModelStatus,
} from '../store/modelSlice';

const POLL_INTERVAL = 1000;

/**
 * App-level provider that auto-starts model download on desktop
 * and keeps Redux model state in sync with the Rust backend.
 */
const ModelProvider = ({ children }: { children: React.ReactNode }) => {
  const dispatch = useAppDispatch();
  const loading = useAppSelector(state => state.model.loading);
  const downloadTriggered = useAppSelector(state => state.model.downloadTriggered);

  // Single init effect: fetch status → check platform → auto-download if needed.
  // No ref guard — safe to re-run; Rust backend prevents concurrent downloads.
  useEffect(() => {
    let cancelled = false;

    const init = async () => {
      // 1. Fetch initial status
      let status: ModelStatus;
      try {
        status = await invoke<ModelStatus>('model_get_status');
        console.log('[ModelProvider] Initial status:', JSON.stringify(status));
        if (cancelled) return;
        dispatch(setModelStatus(status));
      } catch (err) {
        console.log('[ModelProvider] Not in Tauri environment:', err);
        return;
      }

      // 2. Check availability
      try {
        const avail = await invoke<boolean>('model_is_available');
        console.log('[ModelProvider] Available:', avail);
        if (!avail || cancelled) return;
        status = await invoke<ModelStatus>('model_get_status');
        if (cancelled) return;
        dispatch(setModelStatus(status));
      } catch (err) {
        console.log('[ModelProvider] Availability check failed:', err);
        return;
      }

      // 3. If already downloaded or already loading, nothing to do
      if (status.downloaded) {
        console.log('[ModelProvider] Already downloaded, skipping auto-download');
        return;
      }
      if (status.loading) {
        console.log('[ModelProvider] Already loading, will poll');
        if (!cancelled) dispatch(setModelLoading(true));
        return;
      }

      // 4. Check platform — only auto-download on desktop
      try {
        const currentPlatform = await platform();
        console.log('[ModelProvider] Platform:', currentPlatform);
        if (currentPlatform === 'android' || currentPlatform === 'ios') {
          console.log('[ModelProvider] Mobile platform, skipping');
          return;
        }
      } catch (err) {
        console.log('[ModelProvider] Platform detection failed (web?), skipping:', err);
        return;
      }

      if (cancelled) return;

      // 5. Start download
      console.log('[ModelProvider] Starting auto-download...');
      dispatch(setDownloadTriggered(true));
      dispatch(setModelLoading(true));
      dispatch(setModelError(null));

      try {
        await invoke('model_start_download');
        if (cancelled) return;
        const finalStatus = await invoke<ModelStatus>('model_get_status');
        console.log('[ModelProvider] Download complete:', JSON.stringify(finalStatus));
        if (!cancelled) dispatch(setModelStatus(finalStatus));
      } catch (err) {
        console.error('[ModelProvider] Download failed:', err);
        if (!cancelled) dispatch(setModelError(err instanceof Error ? err.message : String(err)));
      }
    };

    // Only run if download hasn't been triggered yet (Redux state, survives StrictMode)
    if (!downloadTriggered) {
      init();
    }

    return () => {
      cancelled = true;
    };
  }, [dispatch, downloadTriggered]);

  // Poll status while loading/downloading
  useEffect(() => {
    if (!loading) return;

    console.log('[ModelProvider] Polling started');
    const interval = setInterval(async () => {
      try {
        const status = await invoke<ModelStatus>('model_get_status');
        dispatch(setModelStatus(status));
        if (!status.loading) {
          console.log('[ModelProvider] Loading finished:', JSON.stringify(status));
        }
      } catch {
        // ignore
      }
    }, POLL_INTERVAL);

    return () => {
      console.log('[ModelProvider] Polling stopped');
      clearInterval(interval);
    };
  }, [dispatch, loading]);

  return <>{children}</>;
};

export default ModelProvider;
