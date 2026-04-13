import debug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';

import { useCoreState } from '../providers/CoreStateProvider';
import {
  type ComposioTriggerHistoryEntry,
  openhumanComposioListTriggerHistory,
} from '../utils/tauriCommands';

const log = debug('composio:history');
const POLL_MS = 5000;

export interface ComposeioTriggerHistoryState {
  archiveDir: string | null;
  currentDayFile: string | null;
  entries: ComposioTriggerHistoryEntry[];
  loading: boolean;
  error: string | null;
  coreConnected: boolean;
  refresh: () => Promise<void>;
}

export function useComposeioTriggerHistory(limit = 100): ComposeioTriggerHistoryState {
  const { snapshot } = useCoreState();
  const [archiveDir, setArchiveDir] = useState<string | null>(null);
  const [currentDayFile, setCurrentDayFile] = useState<string | null>(null);
  const [entries, setEntries] = useState<ComposioTriggerHistoryEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [coreConnected, setCoreConnected] = useState(false);
  const isRefreshingRef = useRef(false);
  const sessionTokenRef = useRef(snapshot.sessionToken);

  const clearHistory = useCallback(() => {
    setArchiveDir(null);
    setCurrentDayFile(null);
    setEntries([]);
    setLoading(false);
    setError(null);
    setCoreConnected(false);
  }, []);

  useEffect(() => {
    sessionTokenRef.current = snapshot.sessionToken;
  }, [snapshot.sessionToken]);

  const refresh = useCallback(async () => {
    if (isRefreshingRef.current) {
      return;
    }
    if (!snapshot.sessionToken) {
      clearHistory();
      return;
    }

    const requestToken = snapshot.sessionToken;
    isRefreshingRef.current = true;
    setLoading(true);
    try {
      const response = await openhumanComposioListTriggerHistory(limit);
      if (!sessionTokenRef.current || sessionTokenRef.current !== requestToken) {
        return;
      }
      const result = response.result.result;
      setArchiveDir(result.archive_dir);
      setCurrentDayFile(result.current_day_file);
      setEntries(result.entries);
      setError(null);
      setCoreConnected(true);
      log('loaded %d composio trigger entries', result.entries.length);
    } catch (refreshError) {
      if (!sessionTokenRef.current || sessionTokenRef.current !== requestToken) {
        return;
      }
      const message =
        refreshError instanceof Error ? refreshError.message : 'Failed to load ComposeIO history';
      setError(message);
      setCoreConnected(false);
      log('failed to load trigger history: %s', message);
    } finally {
      isRefreshingRef.current = false;
      setLoading(false);
    }
  }, [clearHistory, limit, snapshot.sessionToken]);

  useEffect(() => {
    if (snapshot.sessionToken) {
      return;
    }

    clearHistory();
  }, [clearHistory, snapshot.sessionToken]);

  useEffect(() => {
    if (!snapshot.sessionToken) {
      clearHistory();
      return;
    }

    void refresh();
    const timer = window.setInterval(() => {
      void refresh();
    }, POLL_MS);

    return () => {
      window.clearInterval(timer);
    };
  }, [clearHistory, refresh, snapshot.sessionToken]);

  return { archiveDir, currentDayFile, entries, loading, error, coreConnected, refresh };
}
