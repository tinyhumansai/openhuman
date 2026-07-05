/**
 * useSubconscious — hook for the subconscious engine UI.
 *
 * Provides status, mode control, and engine actions for the
 * subconscious tab on the Intelligence page.
 */
import { useCallback, useEffect, useRef, useState } from 'react';

import {
  isTauri,
  openhumanHeartbeatSettingsGet,
  openhumanHeartbeatSettingsSet,
  subconsciousStatus,
  subconsciousTrigger,
} from '../utils/tauriCommands';
import type { SubconsciousMode } from '../utils/tauriCommands/heartbeat';
import type {
  SubconsciousInstanceStatus,
  SubconsciousKind,
  SubconsciousStatus,
} from '../utils/tauriCommands/subconscious';

export type TriggerKind = SubconsciousKind | 'all';

export interface UseSubconsciousResult {
  status: SubconsciousStatus | null;
  /** One row per registered world, tolerant of an older core (falls back to
   * the top-level memory fields when `instances` is absent). */
  instances: SubconsciousInstanceStatus[];
  mode: SubconsciousMode;
  intervalMinutes: number;
  loading: boolean;
  /** True when the memory (default) tick is in flight — back-compat. */
  triggering: boolean;
  /** True when a tick for `kind` is in flight (two buttons ≠ one spinner). */
  isTriggering: (kind: TriggerKind) => boolean;
  settingMode: boolean;
  refresh: () => Promise<void>;
  triggerTick: (kind?: TriggerKind) => Promise<void>;
  setMode: (mode: SubconsciousMode) => Promise<void>;
  setIntervalMinutes: (minutes: number) => Promise<void>;
  error: string | null;
}

/** Derive per-world rows, tolerating an older core that omits `instances`. */
function deriveInstances(status: SubconsciousStatus | null): SubconsciousInstanceStatus[] {
  if (!status) return [];
  if (status.instances && status.instances.length > 0) return status.instances;
  // Older core: the top-level fields are the memory instance.
  const { instances: _omit, ...row } = status;
  return [{ ...row, instance: row.instance ?? 'memory' }];
}

export function useSubconscious(): UseSubconsciousResult {
  const [status, setStatus] = useState<SubconsciousStatus | null>(null);
  const [mode, setModeState] = useState<SubconsciousMode>('off');
  const [intervalMinutes, setIntervalState] = useState(30);
  const [loading, setLoading] = useState(false);
  const [triggeringKinds, setTriggeringKinds] = useState<Set<TriggerKind>>(new Set());
  const [settingMode, setSettingMode] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const fetchingRef = useRef(false);

  const refresh = useCallback(async () => {
    if (!isTauri() || fetchingRef.current) return;
    fetchingRef.current = true;
    setLoading(true);
    setError(null);
    try {
      const [statusRes, settingsRes] = await Promise.all([
        withTimeout(subconsciousStatus()),
        withTimeout(openhumanHeartbeatSettingsGet()),
      ]);
      if (statusRes) setStatus(unwrap(statusRes) ?? null);
      const settings = settingsRes
        ? unwrap<{ settings: { subconscious_mode: SubconsciousMode; interval_minutes: number } }>(
            settingsRes
          )
        : null;
      if (settings?.settings) {
        if (settings.settings.subconscious_mode) {
          setModeState(settings.settings.subconscious_mode);
        }
        if (settings.settings.interval_minutes) {
          setIntervalState(settings.settings.interval_minutes);
        }
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to load subconscious data');
    } finally {
      setLoading(false);
      fetchingRef.current = false;
    }
  }, []);

  const triggerTick = useCallback(async (kind: TriggerKind = 'memory') => {
    if (!isTauri()) return;
    let alreadyInFlight = false;
    setTriggeringKinds(prev => {
      if (prev.has(kind)) {
        alreadyInFlight = true;
        return prev;
      }
      const next = new Set(prev);
      next.add(kind);
      return next;
    });
    if (alreadyInFlight) return;
    try {
      // A no-arg legacy call maps to memory; pass the kind through otherwise.
      await subconsciousTrigger(kind === 'memory' ? undefined : kind);
    } catch (err) {
      console.warn('[subconscious] trigger failed:', err);
    } finally {
      setTriggeringKinds(prev => {
        const next = new Set(prev);
        next.delete(kind);
        return next;
      });
    }
  }, []);

  const setMode = useCallback(
    async (newMode: SubconsciousMode) => {
      if (!isTauri()) return;
      setSettingMode(true);
      setModeState(newMode);
      try {
        await openhumanHeartbeatSettingsSet({ subconscious_mode: newMode });
        await refresh();
      } catch (err) {
        console.warn('[subconscious] setMode failed:', err);
        setError(err instanceof Error ? err.message : 'Failed to update mode');
      } finally {
        setSettingMode(false);
      }
    },
    [refresh]
  );

  const setIntervalMinutes = useCallback(async (minutes: number) => {
    if (!isTauri()) return;
    setIntervalState(minutes);
    try {
      await openhumanHeartbeatSettingsSet({ interval_minutes: minutes });
    } catch (err) {
      console.warn('[subconscious] setInterval failed:', err);
    }
  }, []);

  useEffect(() => {
    refresh();
    const interval = setInterval(refresh, 5000);
    return () => {
      clearInterval(interval);
      fetchingRef.current = false;
    };
  }, [refresh]);

  const isTriggering = useCallback(
    (kind: TriggerKind) => triggeringKinds.has(kind),
    [triggeringKinds]
  );

  return {
    status,
    instances: deriveInstances(status),
    mode,
    intervalMinutes,
    loading,
    triggering: triggeringKinds.has('memory'),
    isTriggering,
    settingMode,
    refresh,
    triggerTick,
    setMode,
    setIntervalMinutes,
    error,
  };
}

const RPC_TIMEOUT_MS = 2500;

function withTimeout<T>(promise: Promise<T>, ms: number = RPC_TIMEOUT_MS): Promise<T | null> {
  return Promise.race<T | null>([
    promise.catch(() => null),
    new Promise<null>(resolve => setTimeout(() => resolve(null), ms)),
  ]);
}

function unwrap<T>(response: unknown): T | null {
  if (!response || typeof response !== 'object') return null;
  const r = response as Record<string, unknown>;
  if ('result' in r) {
    return r.result as T;
  }
  return null;
}
