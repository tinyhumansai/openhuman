/**
 * useFlowRunProgress (Phase 3e — live run overlay)
 * ------------------------------------------------
 *
 * Subscribes to the core's live per-step progress feed for a single durable
 * `tinyflows` run and yields a `node_id -> status` map so the canvas can animate
 * nodes as they execute (n8n's signature running/success/error interaction).
 *
 * The backend's `FlowRunObserver` publishes `DomainEvent::FlowRunProgress` on
 * each finished step; the core socket bridge (`src/core/socketio.rs`) re-emits it
 * to the frontend as **both** `flow:run_progress` and `flow_run_progress`
 * (colon + underscore aliases, same as every other bridged event) with the
 * payload `{ run_id, node_id, status }`.
 *
 * This is a *live overlay only* — the durable `flow_runs` row remains the source
 * of truth and {@link useFlowRunPoller} stays as the 2s fallback, so a dropped
 * broadcast (lag) merely delays the animation, never corrupts run history. The
 * subscription mirrors {@link useTinyplaceStream} exactly (socketService.on/off
 * with cleanup on unmount / dependency change).
 */
import debug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';

import { socketService } from '../services/socketService';

const log = debug('flows:run-progress');

/** Socket event aliases the core bridge emits (colon + underscore forms). */
const EVENT_COLON = 'flow:run_progress';
const EVENT_UNDERSCORE = 'flow_run_progress';

/**
 * Node-level live status. The observer today emits only `success`/`error` on
 * step finish; `running` is included so the hook stays forward-compatible with
 * a future step-start event (and so callers can optimistically mark a node
 * active). Any unrecognized status string is passed through verbatim.
 */
export type FlowNodeRunStatus = 'running' | 'success' | 'error' | (string & {});

/** node_id → latest live status for the watched run. */
export type FlowRunProgressMap = Record<string, FlowNodeRunStatus>;

/**
 * Maps a live node status to the canvas CSS class that rings/animates the node.
 * Kept here (not in the CSS-adjacent component) so the hook, the canvas, and
 * tests share one source of truth. `error` deliberately uses a run-specific
 * class distinct from validation's `.flow-node-error` so a *runtime* failure
 * reads differently from a *config* error.
 */
export const FLOW_RUN_NODE_STATUS_CLASS: Record<string, string> = {
  running: 'flow-node-running',
  success: 'flow-node-success',
  error: 'flow-node-failed',
  failed: 'flow-node-failed',
};

interface FlowRunProgressPayload {
  run_id: string;
  node_id: string;
  status: string;
}

function parsePayload(data: unknown): FlowRunProgressPayload | null {
  if (!data || typeof data !== 'object') return null;
  const obj = data as Record<string, unknown>;
  if (typeof obj.run_id !== 'string') return null;
  if (typeof obj.node_id !== 'string') return null;
  if (typeof obj.status !== 'string') return null;
  return { run_id: obj.run_id, node_id: obj.node_id, status: obj.status };
}

/**
 * Watch `runId`'s live progress. Returns a `node_id -> status` map that grows
 * as steps finish. Yields an empty map (and subscribes to nothing) when `runId`
 * is `null`. Resets whenever `runId` changes so a stale run's node states never
 * bleed onto a newly-started one.
 */
export function useFlowRunProgress(runId: string | null): FlowRunProgressMap {
  const [statuses, setStatuses] = useState<FlowRunProgressMap>({});

  // Reset during render (not synchronously inside the effect below —
  // `react-hooks/set-state-in-effect` disallows that) when `runId` changes, so
  // a stale run's node states never bleed onto a newly-started one.
  const prevRunIdRef = useRef(runId);
  if (prevRunIdRef.current !== runId) {
    prevRunIdRef.current = runId;
    setStatuses({});
  }

  const handleProgress = useCallback(
    (data: unknown) => {
      if (!runId) return;
      const payload = parsePayload(data);
      if (!payload) {
        log('progress: dropped — invalid payload %o', data);
        return;
      }
      // Filter to the run this hook instance is watching; the bridge broadcasts
      // every run's progress to all listeners.
      if (payload.run_id !== runId) return;
      log('progress: run=%s node=%s status=%s', runId, payload.node_id, payload.status);
      setStatuses(prev =>
        prev[payload.node_id] === payload.status
          ? prev
          : { ...prev, [payload.node_id]: payload.status }
      );
    },
    [runId]
  );

  useEffect(() => {
    if (!runId) return;
    log('subscribe: run=%s', runId);
    socketService.on(EVENT_COLON, handleProgress);
    socketService.on(EVENT_UNDERSCORE, handleProgress);
    return () => {
      log('unsubscribe: run=%s', runId);
      socketService.off(EVENT_COLON, handleProgress);
      socketService.off(EVENT_UNDERSCORE, handleProgress);
    };
  }, [runId, handleProgress]);

  return statuses;
}

export default useFlowRunProgress;
