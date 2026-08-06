/**
 * useFlowPendingApprovals (flow-approval surface — run details)
 * ---------------------------------------------------------------
 *
 * Feeds the actionable approval cards in `FlowRunInspectorDrawer`. The core
 * has no dedicated "pending approvals for this run" endpoint — approvals are
 * a single shared queue (`openhuman.approval_list_pending`) covering chat,
 * flow, and any future origin. The module-scoped approval source polls that
 * queue once for every active consumer, and this hook filters its snapshot to
 * one flow run via `PendingApproval.source_context`.
 *
 * Mirrors the poll-until-told-to-stop shape of `useFlowRunPoller`: the caller
 * controls the polling window by passing `null` for either `flowId` or
 * `runId` once the run leaves an active state (`running` /
 * `pending_approval`) — this hook does not know about run status itself.
 *
 * `decide()` wraps `approvalApi.decideApproval` and immediately refreshes the
 * shared queue on success so every approval surface reconciles together. On
 * success the run itself proceeds server-side —
 * `useFlowRunPoller`'s independent 2s loop is what picks up the new steps.
 */
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import {
  type ApprovalDecision,
  decideApproval,
  type PendingApproval,
} from '../services/api/approvalApi';
import {
  refreshFlowPendingApprovals,
  useFlowPendingApprovalsSource,
} from './flowPendingApprovalsStore';

function matchesRun(approval: PendingApproval, flowId: string, runId: string): boolean {
  const ctx = approval.source_context;
  return !!ctx && ctx.kind === 'flow' && ctx.flow_id === flowId && ctx.run_id === runId;
}

interface UseFlowPendingApprovalsResult {
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
 * Retain the shared pending-approval poller while both `flowId` and `runId`
 * are non-null, and select approvals belonging to that run.
 */
export function useFlowPendingApprovals(
  flowId: string | null,
  runId: string | null
): UseFlowPendingApprovalsResult {
  const [decidingId, setDecidingId] = useState<string | null>(null);
  const [decisionError, setDecisionError] = useState<string | null>(null);
  const enabled = !!flowId && !!runId;
  const source = useFlowPendingApprovalsSource(enabled);

  const approvals = useMemo(
    () => (enabled ? source.approvals.filter(approval => matchesRun(approval, flowId, runId)) : []),
    [enabled, flowId, runId, source.approvals]
  );

  const mountedRef = useRef(true);
  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  useEffect(() => setDecisionError(null), [flowId, runId]);

  const decide = useCallback(async (requestId: string, decision: ApprovalDecision) => {
    setDecidingId(requestId);
    setDecisionError(null);
    try {
      await decideApproval(requestId, decision);
      await refreshFlowPendingApprovals();
    } catch (err) {
      if (mountedRef.current) {
        setDecisionError(err instanceof Error ? err.message : String(err));
      }
      throw err;
    } finally {
      if (mountedRef.current) setDecidingId(null);
    }
  }, []);

  return { approvals, decidingId, error: decisionError ?? (enabled ? source.error : null), decide };
}
