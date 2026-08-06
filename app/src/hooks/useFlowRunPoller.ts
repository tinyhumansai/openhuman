/**
 * useFlowRunPoller (issue B3b)
 * ----------------------------
 *
 * Poll-until-terminal loop for a single durable `tinyflows` run, feeding the
 * {@link FlowRunInspectorDrawer}. This is NOT purely a terminal-transition
 * signal — unlike `useFlowRunsLiveRefresh` (list-level, now backstopped by
 * `DomainEvent::FlowRunFinished` via `useFlowRunFinished`), each tick here
 * also refreshes the run's evolving per-step output (`FlowRunStep.output`)
 * for the drawer's step timeline, and no event carries that content — so
 * this hook still needs an actual poll loop while a run is in flight, not
 * just a long backstop. It mirrors the setTimeout-chained poll loop in
 * `components/intelligence/IntelligenceOrchestrationTab.tsx` (~lines 112-143):
 * schedule the next poll only after the current one resolves and the run is
 * still non-terminal, guard against races with `cancelled`/`inFlight`, and
 * never let an unmounted component call `setState`.
 *
 * Now that `DomainEvent::FlowRunFinished` exists (issue B35 follow-up,
 * bridged as `flow:run_finished` / `flow_run_finished`), this hook also
 * subscribes via `useFlowRunFinished` and forces an immediate out-of-cadence
 * tick (cancelling any pending scheduled poll) the moment a finish event for
 * `runId` arrives, rather than waiting up to {@link POLL_INTERVAL_MS} for the
 * next scheduled tick to notice the terminal row. The interval itself is
 * relaxed from 2s to 3s accordingly — it's now a freshness cadence for
 * in-flight step output, not the primary way termination is detected.
 *
 * `pending_approval` is explicitly NOT terminal — a paused run still needs
 * live status so the drawer reflects an approval elsewhere resolving it.
 * `completed_with_warnings` (run honesty, PR2), `cancelled`, and `interrupted`
 * (bug B42 — reconciled after being dropped mid-flight) are terminal, same as
 * `completed`/`failed`.
 */
import debug from 'debug';
import { useEffect, useRef, useState } from 'react';

import { type FlowRun, type FlowRunStatus, getFlowRun } from '../services/api/flowsApi';
import { useFlowRunFinished } from './useFlowRunFinished';

const log = debug('flows:poller');

/**
 * How often to poll a non-terminal run for progress. Relaxed from 2s to 3s
 * now that a `FlowRunFinished` event can force an immediate out-of-cadence
 * tick the moment a run actually settles (see module doc above) — this
 * interval only governs the freshness of in-flight step output between
 * terminal events.
 */
const POLL_INTERVAL_MS = 3000;

const TERMINAL = new Set<FlowRunStatus>([
  'completed',
  'completed_with_warnings',
  'failed',
  'cancelled',
  // Reconciled after its future was dropped mid-flight (bug B42) — a settled
  // terminal state. Without this the poller would loop forever on a run that
  // never leaves `interrupted`.
  'interrupted',
]);

function isTerminal(run: FlowRun | null): boolean {
  return run !== null && TERMINAL.has(run.status);
}

interface UseFlowRunPollerResult {
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

  // Set by the runId effect below to an out-of-cadence "tick now" callback,
  // so the `useFlowRunFinished` subscription (which lives outside that
  // effect, since it must stay mounted across `runId` changes) can force an
  // immediate refetch instead of waiting for the next scheduled poll. Reset
  // to `null` whenever the effect tears down so a finish event that arrives
  // after `runId` changes (or on unmount) can't reach into a torn-down tick.
  const forceTickRef = useRef<(() => void) | null>(null);

  useFlowRunFinished(event => {
    if (event.run_id !== runId) return;
    log(
      'finished event received for runId=%s status=%s — forcing immediate tick',
      runId,
      event.status
    );
    forceTickRef.current?.();
  });

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

    // Let the `useFlowRunFinished` subscription above force an immediate
    // out-of-cadence tick for this `runId`: cancel any pending scheduled
    // poll first so the forced tick and the regular cadence never race into
    // a double in-flight fetch.
    forceTickRef.current = () => {
      if (pollHandle !== undefined) {
        window.clearTimeout(pollHandle);
        pollHandle = undefined;
      }
      void tick();
    };

    void tick();
    return () => {
      cancelled = true;
      if (pollHandle !== undefined) window.clearTimeout(pollHandle);
      forceTickRef.current = null;
    };
  }, [runId]);

  return { run, loading, error };
}
