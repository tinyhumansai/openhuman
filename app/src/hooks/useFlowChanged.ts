/**
 * useFlowChanged (Phase 3 — flow-mutation observability, audit F6)
 * ----------------------------------------------------------------
 *
 * Subscribes to the core's flow-mutation feed so an open Workflows list or
 * canvas reacts when a flow's definition changes underneath it — most
 * importantly, so an agent `save_workflow` becomes visible instead of the user
 * silently working against (and later clobbering) stale state.
 *
 * The backend publishes `DomainEvent::FlowChanged` on create/update/delete/
 * enable; the core socket bridge (`src/core/socketio.rs`) re-emits it as both
 * `flow:changed` and `flow_changed` (colon + underscore aliases) with the
 * payload `{ flow_id, kind, actor }`.
 *
 * Best-effort (broadcast bridges drop on lag); the UI's own refetch-on-focus
 * remains the backstop, exactly as {@link useFlowRunProgress} pairs with the 2s
 * poller. Pass `flowId` to filter to a single flow (canvas), or omit it to
 * receive every change (list).
 */
import debug from 'debug';
import { useCallback, useEffect } from 'react';

import { socketService } from '../services/socketService';

const log = debug('flows:changed');

/** Socket event aliases the core bridge emits (colon + underscore forms). */
const EVENT_COLON = 'flow:changed';
const EVENT_UNDERSCORE = 'flow_changed';

/** What happened to the flow. */
export type FlowChangeKind = 'created' | 'updated' | 'deleted' | 'enabled_changed' | (string & {});

/** Payload of a `flow:changed` socket event (`DomainEvent::FlowChanged`). */
export interface FlowChangedEvent {
  flow_id: string;
  kind: FlowChangeKind;
  /** Coarse hint: `agent` | `user` | `system`. */
  actor: string;
}

function parsePayload(data: unknown): FlowChangedEvent | null {
  if (!data || typeof data !== 'object') return null;
  const obj = data as Record<string, unknown>;
  if (typeof obj.flow_id !== 'string' || typeof obj.kind !== 'string') return null;
  return {
    flow_id: obj.flow_id,
    kind: obj.kind,
    actor: typeof obj.actor === 'string' ? obj.actor : 'system',
  };
}

/**
 * Invokes `onChange` whenever a flow changes. When `flowId` is provided, only
 * changes to that flow are delivered; otherwise every change is.
 */
export function useFlowChanged(
  onChange: (event: FlowChangedEvent) => void,
  flowId?: string | null
): void {
  const handle = useCallback(
    (data: unknown) => {
      const payload = parsePayload(data);
      if (!payload) {
        log('changed: dropped — invalid payload %o', data);
        return;
      }
      if (flowId && payload.flow_id !== flowId) return;
      log('changed: flow=%s kind=%s actor=%s', payload.flow_id, payload.kind, payload.actor);
      onChange(payload);
    },
    [onChange, flowId]
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

export default useFlowChanged;
