import debug from 'debug';
import { useCallback, useEffect, useState } from 'react';

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

  const refresh = useCallback(async () => {
    setLoading(true);
    try {
      const response = await openhumanComposioListTriggerHistory(limit);
      const result = response.result.result;
      setArchiveDir(result.archive_dir);
      setCurrentDayFile(result.current_day_file);
      setEntries(result.entries);
      setError(null);
      setCoreConnected(true);
      log('loaded %d composio trigger entries', result.entries.length);
    } catch (refreshError) {
      const message =
        refreshError instanceof Error ? refreshError.message : 'Failed to load ComposeIO history';
      setError(message);
      setCoreConnected(false);
      log('failed to load trigger history: %s', message);
    } finally {
      setLoading(false);
    }
  }, [limit]);

  useEffect(() => {
    if (!snapshot.sessionToken) {
      return;
    }

    void refresh();
    const timer = window.setInterval(() => {
      void refresh();
    }, POLL_MS);

    return () => {
      window.clearInterval(timer);
    };
  }, [snapshot.sessionToken, refresh]);

  return { archiveDir, currentDayFile, entries, loading, error, coreConnected, refresh };
}
