/**
 * useSubconscious — hook for the subconscious engine UI.
 *
 * Provides tasks, escalations, execution log, and actions for the
 * subconscious tab on the Intelligence page.
 */
import { useCallback, useEffect, useRef, useState } from 'react';

import {
  isTauri,
  subconsciousEscalationsApprove,
  subconsciousEscalationsDismiss,
  subconsciousEscalationsList,
  subconsciousLogList,
  subconsciousStatus,
  subconsciousTasksAdd,
  subconsciousTasksList,
  subconsciousTasksRemove,
  subconsciousTasksUpdate,
  subconsciousTrigger,
} from '../utils/tauriCommands';
import type {
  SubconsciousEscalation,
  SubconsciousLogEntry,
  SubconsciousStatus,
  SubconsciousTask,
} from '../utils/tauriCommands/subconscious';

export interface UseSubconsciousResult {
  // Data
  tasks: SubconsciousTask[];
  escalations: SubconsciousEscalation[];
  logEntries: SubconsciousLogEntry[];
  status: SubconsciousStatus | null;

  // Loading states
  loading: boolean;
  triggering: boolean;

  // Actions
  refresh: () => Promise<void>;
  triggerTick: () => Promise<void>;
  addTask: (title: string) => Promise<void>;
  removeTask: (taskId: string) => Promise<void>;
  toggleTask: (taskId: string, enabled: boolean) => Promise<void>;
  approveEscalation: (escalationId: string) => Promise<void>;
  dismissEscalation: (escalationId: string) => Promise<void>;

  // Error
  error: string | null;
}

export function useSubconscious(): UseSubconsciousResult {
  const [tasks, setTasks] = useState<SubconsciousTask[]>([]);
  const [escalations, setEscalations] = useState<SubconsciousEscalation[]>([]);
  const [logEntries, setLogEntries] = useState<SubconsciousLogEntry[]>([]);
  const [status, setStatus] = useState<SubconsciousStatus | null>(null);
  const [loading, setLoading] = useState(false);
  const [triggering, setTriggering] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const fetchingRef = useRef(false);

  const refresh = useCallback(async () => {
    if (!isTauri() || fetchingRef.current) return;
    fetchingRef.current = true;
    setLoading(true);
    setError(null);
    try {
      const [tasksRes, escalationsRes, logRes, statusRes] = await Promise.all([
        subconsciousTasksList().catch(() => null),
        subconsciousEscalationsList('pending').catch(() => null),
        subconsciousLogList(undefined, 30).catch(() => null),
        subconsciousStatus().catch(() => null),
      ]);

      if (tasksRes) setTasks(unwrap(tasksRes) ?? []);
      if (escalationsRes) setEscalations(unwrap(escalationsRes) ?? []);
      if (logRes) setLogEntries(unwrap(logRes) ?? []);
      if (statusRes) setStatus(unwrap(statusRes) ?? null);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to load subconscious data');
    } finally {
      setLoading(false);
      fetchingRef.current = false;
    }
  }, []);

  const triggerTick = useCallback(async () => {
    if (!isTauri() || triggering) return;
    setTriggering(true);
    try {
      await subconsciousTrigger();
    } catch (err) {
      console.warn('[subconscious] trigger failed:', err);
    } finally {
      setTriggering(false);
    }
  }, [triggering]);

  const addTask = useCallback(
    async (title: string) => {
      if (!isTauri()) return;
      try {
        await subconsciousTasksAdd(title);
        await refresh();
      } catch (err) {
        console.warn('[subconscious] add task failed:', err);
        throw err;
      }
    },
    [refresh]
  );

  const removeTask = useCallback(
    async (taskId: string) => {
      if (!isTauri()) return;
      try {
        await subconsciousTasksRemove(taskId);
        await refresh();
      } catch (err) {
        console.warn('[subconscious] remove task failed:', err);
      }
    },
    [refresh]
  );

  const toggleTask = useCallback(
    async (taskId: string, enabled: boolean) => {
      if (!isTauri()) return;
      try {
        await subconsciousTasksUpdate(taskId, { enabled });
        await refresh();
      } catch (err) {
        console.warn('[subconscious] toggle task failed:', err);
      }
    },
    [refresh]
  );

  const approveEscalation = useCallback(
    async (escalationId: string) => {
      if (!isTauri()) return;
      try {
        await subconsciousEscalationsApprove(escalationId);
        await refresh();
      } catch (err) {
        console.warn('[subconscious] approve failed:', err);
        throw err;
      }
    },
    [refresh]
  );

  const dismissEscalation = useCallback(
    async (escalationId: string) => {
      if (!isTauri()) return;
      try {
        await subconsciousEscalationsDismiss(escalationId);
        await refresh();
      } catch (err) {
        console.warn('[subconscious] dismiss failed:', err);
      }
    },
    [refresh]
  );

  // Poll every 3s while the hook is mounted (user is on Subconscious tab).
  // Picks up all state changes: in_progress → act/noop/escalate/failed,
  // new escalations, background tick completions, etc.
  useEffect(() => {
    refresh();
    const interval = setInterval(refresh, 3000);
    return () => clearInterval(interval);
  }, [refresh]);

  return {
    tasks,
    escalations,
    logEntries,
    status,
    loading,
    triggering,
    refresh,
    triggerTick,
    addTask,
    removeTask,
    toggleTask,
    approveEscalation,
    dismissEscalation,
    error,
  };
}

/**
 * Unwrap a CommandResponse — callCoreRpc returns `{ result: T, logs: [...] }`.
 */
function unwrap<T>(response: unknown): T | null {
  if (!response || typeof response !== 'object') return null;
  const r = response as Record<string, unknown>;
  // CommandResponse shape: { result: T, logs: string[] }
  if ('result' in r) {
    return r.result as T;
  }
  return null;
}
