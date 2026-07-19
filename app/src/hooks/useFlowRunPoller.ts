/**
 * useFlowRunPoller (issue B3b)
 * ----------------------------
 *
 * Poll-until-terminal loop for a single durable `tinyflows` run, feeding the
 * {@link FlowRunInspectorDrawer}. The flows engine emits no socket events for
 * run progress (same situation as the `workflow_run_*` orchestration surface),
 * so this mirrors the setTimeout-chained poll loop in
 * `components/intelligence/IntelligenceOrchestrationTab.tsx` (~lines 112-143):
 * schedule the next poll only after the current one resolves and the run is
 * still non-terminal, guard against races with `cancelled`/`inFlight`, and
 * never let an unmounted component call `setState`.
 *
 * `pending_approval` is explicitly NOT terminal — a paused run still needs
 * live status so the drawer reflects an approval elsewhere resolving it.
 * `completed_with_warnings` (run honesty, PR2) and `cancelled` are terminal,
 * same as `completed`/`failed`.
 */
import debug from 'debug';
import { useEffect, useRef, useState } from 'react';

import { type FlowRun, type FlowRunStatus, getFlowRun } from '../services/api/flowsApi';

const log = debug('flows:poller');

/** How often to poll a non-terminal run for progress. */
const POLL_INTERVAL_MS = 2000;

const TERMINAL = new Set<FlowRunStatus>([
  'completed',
  'completed_with_warnings',
  'failed',
  'cancelled',
]);

function isTerminal(run: FlowRun | null): boolean {
  return run !== null && TERMINAL.has(run.status);
}

export interface UseFlowRunPollerResult {
  run: FlowRun | null;
  loading: boolean;
  error: string | null;
}

/**
 * Poll `openhuman.flows_get_run` for `runId` every {@link POLL_INTERVAL_MS}ms
 * while the run is `running` or `pending_approval`. Stops polling once the
 * run reaches a terminal status, when `runId` becomes `null`, when `runId`
 * changes, or on unmount. A failed fetch surfaces `error` and does NOT
 * schedule another poll — a broken endpoint shouldn't be hammered.
 */
export function useFlowRunPoller(runId: string | null): UseFlowRunPollerResult {
  // Lazy initial state keyed off the `runId` this hook instance first mounts
  // with, so the loading spinner is already correct on the very first paint
  // without a synchronous `setState` in the effect body below.
  const [run, setRun] = useState<FlowRun | null>(null);
  const [loading, setLoading] = useState(() => runId !== null);
  const [error, setError] = useState<string | null>(null);

  const mountedRef = useRef(true);
  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  useEffect(() => {
    // Reset view state for the new target — avoids painting the previous
    // runId's data/error under a different runId while the first fetch for
    // it is in flight. (On the very first mount this just re-applies the
    // lazy-initial values above, so it's a no-op paint-wise.)
    setRun(null);
    setError(null);

    if (!runId) {
      setLoading(false);
      return;
    }
    setLoading(true);

    let cancelled = false;
    let inFlight = false;
    let pollHandle: number | undefined;

    const tick = async () => {
      if (cancelled || inFlight) return;
      inFlight = true;
      try {
        const next = await getFlowRun(runId);
        if (cancelled || !mountedRef.current) return;
        setRun(next);
        setLoading(false);
        setError(null);
        if (!isTerminal(next)) {
          pollHandle = window.setTimeout(() => void tick(), POLL_INTERVAL_MS);
        } else {
          log('tick: runId=%s reached terminal status=%s', runId, next.status);
        }
      } catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        log('tick: error runId=%s err=%s', runId, msg);
        if (cancelled || !mountedRef.current) return;
        setError(msg);
        setLoading(false);
        // Do not schedule another poll — leave retrying to the caller (e.g.
        // reopening the drawer) rather than hammering a broken endpoint.
      } finally {
        inFlight = false;
      }
    };

    void tick();
    return () => {
      cancelled = true;
      if (pollHandle !== undefined) window.clearTimeout(pollHandle);
    };
  }, [runId]);

  return { run, loading, error };
}

export default useFlowRunPoller;
