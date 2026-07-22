/**
 * useFlowRunStarted (issue B35 — runs-rail live refresh)
 * -------------------------------------------------------
 *
 * Subscribes to the core's run-start feed so an open Workflows sidebar/drawer
 * shows a just-started run as "Running" immediately, instead of waiting on a
 * manual refresh or a navigate-away-and-back. `flows_run` is a blocking RPC
 * (up to 610s), so the caller awaiting it can't be the signal — this hook
 * lets the UI learn a run began the moment the `flow_runs` row is persisted,
 * well before the RPC resolves or the first `FlowRunProgress` step lands.
 *
 * The backend publishes `DomainEvent::FlowRunStarted` right after
 * `flows::ops::start_flow_run_row` returns; the core socket bridge
 * (`src/core/socketio.rs`) re-emits it as both `flow:run_started` and
 * `flow_run_started` (colon + underscore aliases) with the payload
 * `{ flow_id, run_id }`.
 *
 * Unlike {@link useFlowRunsLiveRefresh}, this hook subscribes unconditionally
 * (not gated on an already-active run) — that's the whole point: it fills the
 * gap where the runs list is empty ("No runs yet") and so has no active run to
 * gate on. Pass `flowId` to filter to a single flow (canvas/sidebar/drawer),
 * or omit it to receive every run start (the flow-agnostic runs page).
 *
 * Because the core bridge re-emits the same `FlowRunStarted` event under both
 * aliases, this hook subscribes to both sockets above but de-dupes by
 * `${flow_id}:${run_id}` (unique per run) so `onStart` fires exactly once per
 * run — a bounded set of recently-delivered keys (oldest evicted first) is
 * kept per hook instance, sized well past any plausible in-flight burst.
 */
import debug from 'debug';
import { useCallback, useEffect, useRef } from 'react';

import { socketService } from '../services/socketService';

const log = debug('flows:run-started');

/** Socket event aliases the core bridge emits (colon + underscore forms). */
const EVENT_COLON = 'flow:run_started';
const EVENT_UNDERSCORE = 'flow_run_started';

/** Bound on the recently-delivered dedup set — oldest key evicted past this. */
const DEDUP_CACHE_SIZE = 50;

/** Payload of a `flow:run_started` socket event (`DomainEvent::FlowRunStarted`). */
export interface FlowRunStartedEvent {
  flow_id: string;
  run_id: string;
}

function parsePayload(data: unknown): FlowRunStartedEvent | null {
  if (!data || typeof data !== 'object') return null;
  const obj = data as Record<string, unknown>;
  if (typeof obj.flow_id !== 'string' || typeof obj.run_id !== 'string') return null;
  return { flow_id: obj.flow_id, run_id: obj.run_id };
}

/**
 * Invokes `onStart` whenever a run starts. When `flowId` is provided, only
 * starts for that flow are delivered; otherwise every start is.
 */
export function useFlowRunStarted(
  onStart: (event: FlowRunStartedEvent) => void,
  flowId?: string | null
): void {
  // Recently-delivered `${flow_id}:${run_id}` keys, so the colon and
  // underscore aliases of the same event only invoke `onStart` once. A `Set`
  // preserves insertion order, so eviction just drops its first entry.
  const deliveredRef = useRef<Set<string>>(new Set());

  const handle = useCallback(
    (data: unknown) => {
      const payload = parsePayload(data);
      if (!payload) {
        // Never log the raw payload — it may carry PII/secrets. Safe metadata
        // only: the runtime type and, if it's an object, its key names.
        const keys = data && typeof data === 'object' ? Object.keys(data as object) : [];
        log('run-started: dropped — invalid payload (type=%s keys=%o)', typeof data, keys);
        return;
      }
      if (flowId && payload.flow_id !== flowId) return;

      const key = `${payload.flow_id}:${payload.run_id}`;
      const delivered = deliveredRef.current;
      if (delivered.has(key)) {
        log(
          'run-started: dedup skip (alias replay) flow=%s run=%s',
          payload.flow_id,
          payload.run_id
        );
        return;
      }
      delivered.add(key);
      if (delivered.size > DEDUP_CACHE_SIZE) {
        const oldest = delivered.values().next().value;
        if (oldest !== undefined) delivered.delete(oldest);
      }

      log('run-started: flow=%s run=%s', payload.flow_id, payload.run_id);
      onStart(payload);
    },
    [onStart, flowId]
  );

  useEffect(() => {
    socketService.on(EVENT_COLON, handle);
    socketService.on(EVENT_UNDERSCORE, handle);
    return () => {
      socketService.off(EVENT_COLON, handle);
      socketService.off(EVENT_UNDERSCORE, handle);
    };
  }, [handle]);
}

export default useFlowRunStarted;
