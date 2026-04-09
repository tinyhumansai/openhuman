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
      // Each RPC is bounded by RPC_TIMEOUT_MS so Promise.all is guaranteed
      // to settle. Without this, a single hung request (e.g. sidecar held
      // in a long-running tick) would leave fetchingRef.current === true
      // forever, and every subsequent 3s poll would silently no-op at the
      // early-return above — freezing the Intelligence page on a stale
      // snapshot. withTimeout returns null on timeout, matching the
      // existing `.catch(() => null)` failure contract, so downstream
      // setState calls just skip that slice for this tick.
      const [tasksRes, escalationsRes, logRes, statusRes] = await Promise.all([
        withTimeout(subconsciousTasksList()),
        withTimeout(subconsciousEscalationsList('pending')),
        withTimeout(subconsciousLogList(undefined, 30)),
        withTimeout(subconsciousStatus()),
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
  //
  // On unmount we also clear fetchingRef — otherwise a request that times
  // out or resolves after the component has been torn down would leave the
  // ref stuck `true` for the next mount (React Strict Mode double-mount in
  // dev, or tab navigation back to Intelligence), silently wedging the
  // poller exactly as before.
  useEffect(() => {
    refresh();
    const interval = setInterval(refresh, 3000);
    return () => {
      clearInterval(interval);
      fetchingRef.current = false;
    };
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
 * Per-RPC client-side timeout for the polling refresh. Must be strictly
 * less than the 3s poll interval so a hung call can't stack up across
 * ticks. 2500ms leaves a 500ms safety margin.
 */
const RPC_TIMEOUT_MS = 2500;

/**
 * Race a promise against a timeout. Resolves to `null` on timeout or
 * rejection — matching the prior `.catch(() => null)` contract used by
 * the refresh logic so downstream code can treat "no data this tick" and
 * "RPC failed this tick" identically.
 */
function withTimeout<T>(promise: Promise<T>, ms: number = RPC_TIMEOUT_MS): Promise<T | null> {
  return Promise.race<T | null>([
    promise.catch(() => null),
    new Promise<null>(resolve => setTimeout(() => resolve(null), ms)),
  ]);
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
