/**
 * useFlowRunsLiveRefresh — keeps a runs LIST fresh while any run in it is
 * still active, so "Running" doesn't go stale until the user manually
 * refreshes or navigates away.
 *
 * The backend's `FlowRunObserver` publishes `DomainEvent::FlowRunProgress` on
 * each finished step, and the core socket bridge re-emits it to the frontend
 * as both `flow:run_progress` and `flow_run_progress` (colon + underscore
 * aliases — see `useFlowRunProgress`, whose subscribe/teardown style this
 * mirrors). The terminal transition (`running` -> `completed`/`failed`/etc.)
 * now has its own event too — `DomainEvent::FlowRunFinished`, bridged as
 * `flow:run_finished` / `flow_run_finished` — and every caller of this hook
 * (`FlowRunsSidebar`, `FlowRunsDrawer`, `WorkflowRunsPage`) separately wires
 * `useFlowRunFinished` alongside it for that immediate refetch (issue B35
 * follow-up). This hook's own poll is therefore no longer the primary way a
 * terminal transition gets noticed — it's now a long-interval backstop for
 * the (rare) case a broadcast event is dropped under lag or a socket
 * reconnect gap, not a tight polling loop.
 *
 * This is deliberately dumb about *which* run changed — callers pass the
 * list they already fetched and a `refetch` that reloads it; this hook just
 * decides *when* to call `refetch` again. `FlowRunsSidebar`, `FlowRunsDrawer`,
 * and `WorkflowRunsPage` all wire it onto their existing one-shot fetchers.
 */
import debug from 'debug';
import { useEffect, useRef } from 'react';

import type { FlowRun, FlowRunStatus } from '../services/api/flowsApi';
import { socketService } from '../services/socketService';

const log = debug('flows:runs-live-refresh');

/** Socket event aliases the core bridge emits (colon + underscore forms). */
const EVENT_COLON = 'flow:run_progress';
const EVENT_UNDERSCORE = 'flow_run_progress';

/** Statuses a run never leaves once reached — no further refetch needed for it. */
const TERMINAL_STATUSES = new Set<FlowRunStatus>([
  'completed',
  'completed_with_warnings',
  'failed',
  'cancelled',
  // Reconciled after its future was dropped mid-flight (bug B42) — settled, so
  // the list's active-run backstop poll can quiesce once every run is terminal.
  'interrupted',
]);

/** Trailing debounce window for a burst of `flow:run_progress` events. */
const DEBOUNCE_MS = 3_000;

/**
 * Poll fallback cadence. Now a long safety net (not the primary terminal-
 * transition signal — `useFlowRunFinished` is, see module doc above): only
 * matters if a broadcast event was dropped under lag or during a socket
 * reconnect gap.
 */
const POLL_INTERVAL_MS = 30_000;

/**
 * Subscribes to live run-progress events (debounced) and polls on a fallback
 * interval while `runs` contains at least one non-terminal run, calling
 * `refetch` to reload the list. Subscribes to nothing — and tears down any
 * existing subscription/poll — once every run has settled or on unmount.
 */
export function useFlowRunsLiveRefresh(runs: FlowRun[], refetch: () => void): void {
  // Keep the latest `refetch` available to the effect without retriggering
  // subscribe/unsubscribe every time the caller passes a new closure.
  const refetchRef = useRef(refetch);
  refetchRef.current = refetch;

  const hasActive = runs.some(run => !TERMINAL_STATUSES.has(run.status));

  useEffect(() => {
    if (!hasActive) return;

    let debounceTimer: ReturnType<typeof setTimeout> | null = null;
    const scheduleRefetch = () => {
      if (debounceTimer) clearTimeout(debounceTimer);
      debounceTimer = setTimeout(() => {
        debounceTimer = null;
        log('debounced refetch firing');
        refetchRef.current();
      }, DEBOUNCE_MS);
    };

    const handleProgress = () => {
      log('progress event received — scheduling debounced refetch');
      scheduleRefetch();
    };

    log('subscribing: at least one active run');
    socketService.on(EVENT_COLON, handleProgress);
    socketService.on(EVENT_UNDERSCORE, handleProgress);

    const pollId = setInterval(() => {
      log('poll fallback refetch');
      refetchRef.current();
    }, POLL_INTERVAL_MS);

    return () => {
      log('tearing down: unsubscribing + clearing poll/debounce timers');
      socketService.off(EVENT_COLON, handleProgress);
      socketService.off(EVENT_UNDERSCORE, handleProgress);
      clearInterval(pollId);
      if (debounceTimer) clearTimeout(debounceTimer);
    };
  }, [hasActive]);
}
