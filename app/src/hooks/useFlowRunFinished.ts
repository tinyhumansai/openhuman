/**
 * useFlowRunFinished (issue B35 follow-up — runs-rail live refresh)
 * -------------------------------------------------------------------
 *
 * Terminal companion to {@link useFlowRunStarted}: subscribes to the core's
 * run-finish feed so an open Workflows sidebar/drawer flips a run to
 * Completed/Failed the instant it settles, instead of waiting on a poll to
 * notice the terminal `flow_runs` row.
 *
 * The backend publishes `DomainEvent::FlowRunFinished` right after
 * `flows::ops::finish_flow_run_row` persists the settled row; the core socket
 * bridge (`src/core/socketio.rs`) re-emits it as both `flow:run_finished` and
 * `flow_run_finished` (colon + underscore aliases) with the payload
 * `{ flow_id, run_id, status }`.
 *
 * Like `useFlowRunStarted`, this hook subscribes unconditionally (not gated
 * on an already-active run). Pass `flowId` to filter to a single flow
 * (canvas/sidebar/drawer), or omit it to receive every run finish (the
 * flow-agnostic runs page).
 *
 * Because the core bridge re-emits the same `FlowRunFinished` event under
 * both aliases, this hook subscribes to both sockets above but de-dupes by
 * `${flow_id}:${run_id}` (unique per run) so `onFinish` fires exactly once
 * per run — a bounded set of recently-delivered keys (oldest evicted first)
 * is kept per hook instance, sized well past any plausible in-flight burst.
 */
import debug from 'debug';
import { useCallback, useEffect, useRef } from 'react';

import { socketService } from '../services/socketService';

const log = debug('flows:run-finished');

/** Socket event aliases the core bridge emits (colon + underscore forms). */
const EVENT_COLON = 'flow:run_finished';
const EVENT_UNDERSCORE = 'flow_run_finished';

/** Bound on the recently-delivered dedup set — oldest key evicted past this. */
const DEDUP_CACHE_SIZE = 50;

/** Payload of a `flow:run_finished` socket event (`DomainEvent::FlowRunFinished`). */
interface FlowRunFinishedEvent {
  flow_id: string;
  run_id: string;
  status: string;
}

function parsePayload(data: unknown): FlowRunFinishedEvent | null {
  if (!data || typeof data !== 'object') return null;
  const obj = data as Record<string, unknown>;
  if (
    typeof obj.flow_id !== 'string' ||
    typeof obj.run_id !== 'string' ||
    typeof obj.status !== 'string'
  )
    return null;
  return { flow_id: obj.flow_id, run_id: obj.run_id, status: obj.status };
}

/**
 * Invokes `onFinish` whenever a run finishes. When `flowId` is provided,
 * only finishes for that flow are delivered; otherwise every finish is.
 */
export function useFlowRunFinished(
  onFinish: (event: FlowRunFinishedEvent) => void,
  flowId?: string | null
): void {
  // Recently-delivered `${flow_id}:${run_id}` keys, so the colon and
  // underscore aliases of the same event only invoke `onFinish` once. A `Set`
  // preserves insertion order, so eviction just drops its first entry.
  const deliveredRef = useRef<Set<string>>(new Set());

  const handle = useCallback(
    (data: unknown) => {
      const payload = parsePayload(data);
      if (!payload) {
        // Never log the raw payload — it may carry PII/secrets. Safe metadata
        // only: the runtime type and, if it's an object, its key names.
        const keys = data && typeof data === 'object' ? Object.keys(data as object) : [];
        log('run-finished: dropped — invalid payload (type=%s keys=%o)', typeof data, keys);
        return;
      }
      if (flowId && payload.flow_id !== flowId) return;

      const key = `${payload.flow_id}:${payload.run_id}`;
      const delivered = deliveredRef.current;
      if (delivered.has(key)) {
        log(
          'run-finished: dedup skip (alias replay) flow=%s run=%s',
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

      log(
        'run-finished: flow=%s run=%s status=%s',
        payload.flow_id,
        payload.run_id,
        payload.status
      );
      onFinish(payload);
    },
    [onFinish, flowId]
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
