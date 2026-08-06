/**
 * useRunsPendingApprovalSet — cross-references the shared approval queue
 * against a runs LIST so surfaces that show many runs at once (sidebar,
 * drawer, all-runs page) can distinguish a run merely `running` from one
 * that's actually halted at an interactive `ApprovalGate` mid-run.
 *
 * The core has no separate DB status for "parked at an approval gate" — a
 * run stays `status: "running"` in `FlowRun` while it waits; only the shared
 * `openhuman.approval_list_pending` queue (see `useFlowPendingApprovals`,
 * the single-run analogue used by `FlowRunInspectorDrawer`) knows it's
 * parked, via `PendingApproval.source_context = { kind: "flow", run_id }`.
 *
 * While at least one run in the list is `running`, this hook retains the
 * shared pending-approval source and returns the Set of
 * `run_id`s with a matching flow-origin pending approval. Callers combine
 * this with `run.status` via {@link resolveDisplayStatus} to override the
 * DISPLAY status only — `run.status` itself, and `useFlowRunsLiveRefresh`
 * (which reads the ORIGINAL runs array), are untouched, so `running` stays
 * non-terminal there and the list keeps refetching until the run truly
 * finishes.
 *
 * The source owns the poll loop, transient retry, and last-good snapshot.
 * This hook only controls whether its consumer contributes a polling retain.
 */
import { useMemo } from 'react';

import type { FlowRun, FlowRunStatus } from '../services/api/flowsApi';
import { useFlowPendingApprovalsSource } from './flowPendingApprovalsStore';

/**
 * Retain shared polling while `runs` contains a `status === 'running'` entry,
 * and derive the Set of flow-origin pending `run_id`s from its snapshot.
 */
export function useRunsPendingApprovalSet(runs: FlowRun[]): Set<string> {
  const hasRunning = runs.some(run => run.status === 'running');
  const source = useFlowPendingApprovalsSource(hasRunning);

  return useMemo(() => {
    const pendingRunIds = new Set<string>();
    for (const approval of source.approvals) {
      if (approval.source_context?.kind === 'flow') {
        pendingRunIds.add(approval.source_context.run_id);
      }
    }
    return pendingRunIds;
  }, [source.approvals]);
}

/**
 * Overrides a run's DISPLAY status to `pending_approval` when it's currently
 * `running` AND the shared approval queue (via {@link useRunsPendingApprovalSet})
 * has a flow-origin pending approval for it. Does not mutate `run` or touch
 * any other status — callers substitute the result at render sites only
 * (status dot / accent / label), leaving `run.status` itself (and anything
 * keyed on it, like `useFlowRunsLiveRefresh`'s terminal-status check) intact.
 */
export function resolveDisplayStatus(run: FlowRun, pendingRunIds: Set<string>): FlowRunStatus {
  if (run.status === 'running' && pendingRunIds.has(run.id)) {
    return 'pending_approval';
  }
  return run.status;
}
