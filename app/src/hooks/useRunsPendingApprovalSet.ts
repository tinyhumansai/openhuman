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
 * While at least one run in the list is `running`, this hook polls that
 * shared queue every {@link POLL_INTERVAL_MS} and returns the Set of
 * `run_id`s with a matching flow-origin pending approval. Callers combine
 * this with `run.status` via {@link resolveDisplayStatus} to override the
 * DISPLAY status only — `run.status` itself, and `useFlowRunsLiveRefresh`
 * (which reads the ORIGINAL runs array), are untouched, so `running` stays
 * non-terminal there and the list keeps refetching until the run truly
 * finishes.
 *
 * Mirrors the poll-loop + cleanup shape of `useFlowPendingApprovals`
 * (cancelled flag + `setTimeout` reschedule + teardown) and the
 * gate-on-activity contract of `useFlowRunsLiveRefresh`. Unlike
 * `useFlowPendingApprovals`, a failed poll here does NOT surface an error or
 * stop polling — this is a best-effort display hint, not a source of truth,
 * so a transient failure just keeps showing the last-known set and retries
 * on the next tick.
 */
import debug from 'debug';
import { useEffect, useState } from 'react';

import { fetchPendingApprovals } from '../services/api/approvalApi';
import type { FlowRun, FlowRunStatus } from '../services/api/flowsApi';

const log = debug('flows:runs-pending-approval-set');

/** How often to re-poll the shared approval queue while any run is `running`. */
const POLL_INTERVAL_MS = 3000;

/**
 * Poll `openhuman.approval_list_pending` every {@link POLL_INTERVAL_MS}ms
 * while `runs` contains at least one `status === 'running'` entry, and
 * return the Set of `run_id`s with a flow-origin pending approval
 * (`source_context.kind === 'flow'`). Stops polling — but does not clear the
 * last-known set — once every run has left `running` (completed, failed,
 * cancelled, or already reflected as `pending_approval`). Best-effort: a
 * failed poll is logged and simply retried next tick, keeping the
 * last-known set rather than surfacing an error to the caller.
 */
export function useRunsPendingApprovalSet(runs: FlowRun[]): Set<string> {
  const [pendingRunIds, setPendingRunIds] = useState<Set<string>>(() => new Set());

  const hasRunning = runs.some(run => run.status === 'running');

  useEffect(() => {
    if (!hasRunning) {
      log('approval polling skipped: no-running-runs');
      return;
    }
    log('approval polling started');

    let cancelled = false;
    let pollHandle: number | undefined;

    const tick = async () => {
      if (cancelled) return;
      try {
        log('approval poll: calling fetchPendingApprovals');
        const all = await fetchPendingApprovals();
        if (cancelled) return;
        const next = new Set<string>();
        for (const approval of all) {
          if (approval.source_context?.kind === 'flow') {
            next.add(approval.source_context.run_id);
          }
        }
        log('approval poll: succeeded total=%d flow-scoped=%d', all.length, next.size);
        setPendingRunIds(next);
      } catch (err) {
        const errorType = err instanceof Error ? err.name : typeof err;
        log('approval poll: failed error_type=%s; preserving-last-known-set', errorType);
        // Best-effort — leave `pendingRunIds` as-is and retry next tick.
      } finally {
        if (!cancelled) {
          log('approval poll: scheduling retry delay_ms=%d', POLL_INTERVAL_MS);
          pollHandle = window.setTimeout(() => void tick(), POLL_INTERVAL_MS);
        }
      }
    };

    void tick();
    return () => {
      cancelled = true;
      if (pollHandle !== undefined) window.clearTimeout(pollHandle);
      log('approval polling stopped');
    };
  }, [hasRunning]);

  return pendingRunIds;
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

export default useRunsPendingApprovalSet;
