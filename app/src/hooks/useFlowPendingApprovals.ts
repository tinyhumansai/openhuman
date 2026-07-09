/**
 * useFlowPendingApprovals (flow-approval surface — run details)
 * ---------------------------------------------------------------
 *
 * Feeds the actionable approval cards in `FlowRunInspectorDrawer`. The core
 * has no dedicated "pending approvals for this run" endpoint — approvals are
 * a single shared queue (`openhuman.approval_list_pending`) covering chat,
 * flow, and any future origin — so this hook polls that shared queue every
 * ~2s and filters client-side to the gates raised for one specific flow run
 * via `PendingApproval.source_context` (`{ kind: "flow", flow_id, run_id }`).
 *
 * Mirrors the poll-until-told-to-stop shape of `useFlowRunPoller`: the caller
 * controls the polling window by passing `null` for either `flowId` or
 * `runId` once the run leaves an active state (`running` /
 * `pending_approval`) — this hook does not know about run status itself.
 *
 * `decide()` wraps `approvalApi.decideApproval` and optimistically drops the
 * request from the local list on success; the next poll tick reconciles
 * against the server (e.g. a sequential-gate flow re-parking on a fresh
 * request id). On success the run itself proceeds server-side —
 * `useFlowRunPoller`'s independent 2s loop is what picks up the new steps.
 */
import debug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';

import {
  type ApprovalDecision,
  decideApproval,
  fetchPendingApprovals,
  type PendingApproval,
} from '../services/api/approvalApi';

const log = debug('flows:pending-approvals');

/** How often to re-poll the shared approval queue while a run is active. */
const POLL_INTERVAL_MS = 2000;

function matchesRun(approval: PendingApproval, flowId: string, runId: string): boolean {
  const ctx = approval.source_context;
  return !!ctx && ctx.kind === 'flow' && ctx.flow_id === flowId && ctx.run_id === runId;
}

export interface UseFlowPendingApprovalsResult {
  /** Pending approvals scoped to this flow/run, oldest first (server order). */
  approvals: PendingApproval[];
  /** `request_id` of the approval currently being decided, or `null`. */
  decidingId: string | null;
  /** Set when the last poll or decide call failed; cleared on the next success. */
  error: string | null;
  /** Record a decision for one of `approvals`. Throws on failure (caller may ignore). */
  decide: (requestId: string, decision: ApprovalDecision) => Promise<void>;
}

/**
 * Poll `openhuman.approval_list_pending` every {@link POLL_INTERVAL_MS}ms
 * while both `flowId` and `runId` are non-null, filtered to that run via
 * `source_context`. Stops polling (and clears state) when either argument
 * becomes `null` or changes. A failed poll surfaces `error` and does not
 * reschedule — same "don't hammer a broken endpoint" contract as
 * `useFlowRunPoller`; the caller re-enabling polling (e.g. the drawer
 * reopening) is what retries.
 */
export function useFlowPendingApprovals(
  flowId: string | null,
  runId: string | null
): UseFlowPendingApprovalsResult {
  const [approvals, setApprovals] = useState<PendingApproval[]>([]);
  const [decidingId, setDecidingId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const mountedRef = useRef(true);
  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  useEffect(() => {
    setApprovals([]);
    setError(null);

    if (!flowId || !runId) return;

    let cancelled = false;
    let pollHandle: number | undefined;

    const tick = async () => {
      if (cancelled) return;
      try {
        const all = await fetchPendingApprovals();
        if (cancelled || !mountedRef.current) return;
        const scoped = all.filter(a => matchesRun(a, flowId, runId));
        log('tick: flow=%s run=%s total=%d scoped=%d', flowId, runId, all.length, scoped.length);
        setApprovals(scoped);
        setError(null);
        pollHandle = window.setTimeout(() => void tick(), POLL_INTERVAL_MS);
      } catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        log('tick: error flow=%s run=%s err=%s', flowId, runId, msg);
        if (cancelled || !mountedRef.current) return;
        setError(msg);
        // Do not schedule another poll — leave retrying to the caller.
      }
    };

    void tick();
    return () => {
      cancelled = true;
      if (pollHandle !== undefined) window.clearTimeout(pollHandle);
    };
  }, [flowId, runId]);

  const decide = useCallback(async (requestId: string, decision: ApprovalDecision) => {
    setDecidingId(requestId);
    setError(null);
    try {
      await decideApproval(requestId, decision);
      log('decide: ok request=%s decision=%s', requestId, decision);
      if (mountedRef.current) {
        setApprovals(prev => prev.filter(a => a.request_id !== requestId));
      }
    } catch (err) {
      log('decide: failed request=%s err=%o', requestId, err);
      if (mountedRef.current) {
        setError(err instanceof Error ? err.message : String(err));
      }
      throw err;
    } finally {
      if (mountedRef.current) setDecidingId(null);
    }
  }, []);

  return { approvals, decidingId, error, decide };
}

export default useFlowPendingApprovals;
