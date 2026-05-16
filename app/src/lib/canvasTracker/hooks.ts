import { useCallback, useEffect, useMemo, useState } from 'react';

import {
  getCanvasTrackerSettings,
  listCanvasTrackerTasks,
  syncCanvasTrackerNow,
  updateCanvasTaskStatus,
} from './canvasTrackerApi';
import type { CanvasTask, CanvasTrackerSettings, LocalStatus, SyncSummary } from './types';

const urgencyRank: Record<string, number> = {
  critical: 0,
  high: 1,
  medium: 2,
  unclear: 3,
  low: 4,
};

export function sortCanvasTasks(tasks: CanvasTask[]): CanvasTask[] {
  return [...tasks].sort((a, b) => {
    const urgency = (urgencyRank[a.urgency_level] ?? 9) - (urgencyRank[b.urgency_level] ?? 9);
    if (urgency !== 0) return urgency;
    return String(a.due_at ?? '9999').localeCompare(String(b.due_at ?? '9999'));
  });
}

export function useCanvasTracker() {
  const [settings, setSettings] = useState<CanvasTrackerSettings | null>(null);
  const [tasks, setTasks] = useState<CanvasTask[]>([]);
  const [loading, setLoading] = useState(true);
  const [syncing, setSyncing] = useState(false);
  const [lastSync, setLastSync] = useState<SyncSummary | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setError(null);
    try {
      const [nextSettings, nextTasks] = await Promise.all([
        getCanvasTrackerSettings(),
        listCanvasTrackerTasks(),
      ]);
      setSettings(nextSettings);
      setTasks(nextTasks);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const syncNow = useCallback(async () => {
    setSyncing(true);
    setError(null);
    try {
      const summary = await syncCanvasTrackerNow();
      setLastSync(summary);
      await refresh();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSyncing(false);
    }
  }, [refresh]);

  const updateStatus = useCallback(
    async (task: CanvasTask, status: LocalStatus) => {
      await updateCanvasTaskStatus({
        course_id: task.course_id,
        assignment_id: task.assignment_id,
        status,
      });
      await refresh();
    },
    [refresh]
  );

  const sortedTasks = useMemo(() => sortCanvasTasks(tasks), [tasks]);

  return {
    settings,
    tasks: sortedTasks,
    loading,
    syncing,
    lastSync,
    error,
    refresh,
    syncNow,
    updateStatus,
  };
}
