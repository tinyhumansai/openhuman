/**
 * React hook for the Recall.ai Calendar connection surface.
 *
 * Polls connection status (mirrors the Composio connect flow) and auto-flips
 * the core's `meet.calendar_provider` so Google Meet detection follows Recall
 * once connected — and reverts to Composio on disconnect. Provider is core
 * config, so the flip is a client-driven config update.
 */
import { useCallback, useEffect, useRef, useState } from 'react';

import * as recallCalendarApi from './recallCalendarApi';
import { openUrl } from '../../utils/openUrl';
import { openhumanUpdateMeetSettings } from '../../utils/tauriCommands/config';
import type { RecallCalendarStatus } from './recallCalendarApi';

const POLL_INTERVAL_MS = 5000;
type CalendarProvider = 'composio' | 'recall';

export interface UseRecallCalendar {
  status: RecallCalendarStatus | null;
  loading: boolean;
  busy: boolean;
  error: string | null;
  beginConnect: () => Promise<void>;
  disconnect: () => Promise<void>;
  refresh: () => Promise<void>;
}

export function useRecallCalendar(): UseRecallCalendar {
  const [status, setStatus] = useState<RecallCalendarStatus | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const syncedProvider = useRef<CalendarProvider | null>(null);

  const refresh = useCallback(async () => {
    try {
      const next = await recallCalendarApi.status();
      setStatus(next);
      // A successful poll means the fetch itself recovered — clear any stale
      // generic fetch error from a previous blip. Provider-switch errors are
      // owned by the flip branch below (which clears them on its own success),
      // so leave those untouched here.
      setError(current =>
        current && !current.startsWith('calendar provider switch failed:') ? null : current
      );
      const desiredProvider: CalendarProvider =
        next.enabled && next.connected ? 'recall' : 'composio';
      // Keep the local core routing flag in sync even when the first status
      // poll already reports "connected" after an OAuth flow completed outside
      // this mounted component.
      if (syncedProvider.current !== desiredProvider) {
        try {
          await openhumanUpdateMeetSettings({ calendar_provider: desiredProvider });
          syncedProvider.current = desiredProvider;
          setError(current =>
            current?.startsWith('calendar provider switch failed:') ? null : current
          );
        } catch (flipErr) {
          // Non-fatal: the status itself is valid; surface for diagnostics
          // without discarding the connection state.
          setError(`calendar provider switch failed: ${String(flipErr)}`);
        }
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
    // Once the server reports the integration disabled (stable per session),
    // stop live polling — the initial refresh above still learns `enabled`.
    if (status && !status.enabled) return;
    const id = setInterval(() => void refresh(), POLL_INTERVAL_MS);
    return () => clearInterval(id);
  }, [refresh, status?.enabled]);

  const beginConnect = useCallback(async () => {
    setBusy(true);
    setError(null);
    try {
      const { connectUrl } = await recallCalendarApi.connect();
      await openUrl(connectUrl);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }, []);

  const disconnect = useCallback(async () => {
    setBusy(true);
    setError(null);
    try {
      await recallCalendarApi.disconnect();
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }, [refresh]);

  return { status, loading, busy, error, beginConnect, disconnect, refresh };
}
