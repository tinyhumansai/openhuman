/**
 * useFlowRunsLiveRefresh — keeps a runs LIST fresh while any run in it is
 * still active, so "Running" doesn't go stale until the user manually
 * refreshes or navigates away.
 *
 * The backend's `FlowRunObserver` publishes `DomainEvent::FlowRunProgress` on
 * each finished step, and the core socket bridge re-emits it to the frontend
 * as both `flow:run_progress` and `flow_run_progress` (colon + underscore
 * aliases — see `useFlowRunProgress`, whose subscribe/teardown style this
 * mirrors). There is, however, no terminal/completion socket event today —
 * a run transitioning `running` -> `completed`/`failed`/etc. emits nothing —
 * so a step-progress subscription alone can't catch that final refresh. This
 * hook therefore layers a 5s poll fallback on top of the socket subscription:
 * the socket keeps the refetch snappy while steps are landing, and the poll
 * guarantees the terminal transition (which has no event) still gets picked
 * up within a few seconds.
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
]);

/** Trailing debounce window for a burst of `flow:run_progress` events. */
const DEBOUNCE_MS = 3_000;

/** Poll fallback cadence — catches the terminal transition, which has no socket event. */
const POLL_INTERVAL_MS = 5_000;

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

export default useFlowRunsLiveRefresh;
