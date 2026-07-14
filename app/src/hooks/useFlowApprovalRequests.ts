/**
 * useFlowApprovalRequests (flow-approval surface — chat)
 * --------------------------------------------------------
 *
 * Subscribes to the core's `flow_approval_request` socket event — raised
 * when a `tinyflows` run pauses on an approval-gated tool call — and
 * surfaces it as a live list so the chat surface can show an actionable
 * banner without the user having to open the run inspector. Mirrors
 * `useFlowRunProgress`'s subscription shape (`socketService.on`/`off` with
 * cleanup on unmount) and listens for both the colon and underscore aliases
 * the core socket bridge emits for every bridged event.
 *
 * Payload: `{ request_id, flow_id, run_id, tool_name, summary }`. Unlike the
 * thread-scoped `approval_request` chat event (`ApprovalRequestCard`), this
 * event carries no `thread_id` — a flow run isn't tied to the chat thread
 * that's open when the gate fires — so requests are tracked independently
 * of the active thread and surfaced regardless of which thread is selected.
 *
 * Decisions are NOT made here — `decide`/`dismiss` just drop the request off
 * the local list once `FlowApprovalRequestCard` has resolved it via
 * `approvalApi.decideApproval`, so a slow-to-arrive duplicate broadcast (or
 * the same request already decided from another surface, e.g. the run
 * inspector) doesn't linger.
 */
import debug from 'debug';
import { useCallback, useEffect, useState } from 'react';

import { socketService } from '../services/socketService';

const log = debug('flows:approval-requests');

/** Socket event aliases the core bridge emits (colon + underscore forms). */
const EVENT_COLON = 'flow:approval_request';
const EVENT_UNDERSCORE = 'flow_approval_request';

export interface FlowApprovalRequest {
  request_id: string;
  flow_id: string;
  run_id: string;
  tool_name: string;
  summary: string;
}

function parsePayload(data: unknown): FlowApprovalRequest | null {
  if (!data || typeof data !== 'object') return null;
  const obj = data as Record<string, unknown>;
  if (typeof obj.request_id !== 'string') return null;
  if (typeof obj.flow_id !== 'string') return null;
  if (typeof obj.run_id !== 'string') return null;
  if (typeof obj.tool_name !== 'string') return null;
  if (typeof obj.summary !== 'string') return null;
  return {
    request_id: obj.request_id,
    flow_id: obj.flow_id,
    run_id: obj.run_id,
    tool_name: obj.tool_name,
    summary: obj.summary,
  };
}

export interface UseFlowApprovalRequestsResult {
  /** Live flow-approval requests awaiting a decision, oldest first. */
  requests: FlowApprovalRequest[];
  /** Drop a request off the list once it's been decided (or otherwise resolved). */
  dismiss: (requestId: string) => void;
}

/**
 * Watches for `flow_approval_request` broadcasts and keeps a de-duplicated
 * (by `request_id`) list of pending ones for the caller to render.
 */
export function useFlowApprovalRequests(): UseFlowApprovalRequestsResult {
  const [requests, setRequests] = useState<FlowApprovalRequest[]>([]);

  const handleRequest = useCallback((data: unknown) => {
    const parsed = parsePayload(data);
    if (!parsed) {
      log('dropped — invalid payload %o', data);
      return;
    }
    log(
      'request: id=%s flow=%s run=%s tool=%s',
      parsed.request_id,
      parsed.flow_id,
      parsed.run_id,
      parsed.tool_name
    );
    setRequests(prev =>
      prev.some(r => r.request_id === parsed.request_id) ? prev : [...prev, parsed]
    );
  }, []);

  useEffect(() => {
    socketService.on(EVENT_COLON, handleRequest);
    socketService.on(EVENT_UNDERSCORE, handleRequest);
    return () => {
      socketService.off(EVENT_COLON, handleRequest);
      socketService.off(EVENT_UNDERSCORE, handleRequest);
    };
  }, [handleRequest]);

  const dismiss = useCallback((requestId: string) => {
    log('dismiss: id=%s', requestId);
    setRequests(prev => prev.filter(r => r.request_id !== requestId));
  }, []);

  return { requests, dismiss };
}

export default useFlowApprovalRequests;
