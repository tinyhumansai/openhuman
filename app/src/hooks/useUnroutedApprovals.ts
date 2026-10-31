/**
 * useUnroutedApprovals — the approvals no other surface will ever show.
 *
 * The approval gate parks a tool call and waits for a decision. Which surface
 * asks the user for that decision depends on where the call came from:
 *
 *   - a chat turn  -> `ApprovalRequestCard` in the transcript, routed by the
 *                     `approval_request` socket event's `thread_id`
 *   - a flow run   -> `FlowApprovalRequestCard`, routed by the park's
 *                     `source_context`
 *   - anything else-> nothing, until this hook
 *
 * That last case is not hypothetical. A background trigger run (a Composio
 * Gmail trigger reaching `triage.escalate`) has no chat thread and no flow
 * context, so the gate publishes `ApprovalRequested` with `thread_id: None`
 * and `client_id: None`, the web-channel subscriber drops it — its own log
 * line says "NOT surfacing" — and the park sits for its full 600 s TTL and is
 * denied as `[policy-denied]`. The user is never asked. Every escalated email
 * is silently dropped (openhuman#6406; the general form is openhuman#5746).
 *
 * ## Why poll the durable queue instead of listening for an event
 *
 * The defining property of this failure is that **nobody is looking**: the
 * trigger can fire with the app shut. A socket broadcast with no subscriber at
 * that instant is lost exactly as completely as the event the subscriber drops
 * today. `approval_list_pending` is session-agnostic and reads the persisted
 * `pending_approvals` rows, so a park raised while the app was closed is still
 * there when it opens. That is what openhuman#5746's first acceptance bullet
 * asks for in as many words: "an approvals inbox / queue ... rather than only
 * as an event on a bus nobody is listening to at that moment".
 *
 * It is also origin-agnostic. Anything the gate parks and no one claims shows
 * up here — `ExternalChannel` and `TrustedAutomation{Workflow}` parks
 * included, not just triage.
 *
 * ## The discriminator, and which way it fails
 *
 * A pending row carries no `thread_id`, so it cannot say on its own whether a
 * chat surface is already showing it. Two exclusions instead:
 *
 *   - a `source_context` means a flow surface owns it;
 *   - a `request_id` already in `pendingApprovalByThread` means the chat
 *     transcript is already showing its card.
 *
 * Everything else is unclaimed. If that is ever wrong the user sees one
 * approval in two places, which is the direction to be wrong in — the bug
 * being fixed is that they see it in none.
 */
import { useCallback, useMemo, useRef, useState } from 'react';

import {
  type ApprovalDecisionInput,
  decideApproval,
  type PendingApproval,
} from '../services/api/approvalApi';
import { useAppSelector } from '../store/hooks';
import {
  refreshFlowPendingApprovals,
  useFlowPendingApprovalsSource,
} from './flowPendingApprovalsStore';

/**
 * Cadence for this consumer. A flow inspector polls at 2 s because the user is
 * watching a run; a background park has a 600 s TTL and nobody is watching
 * anything, so 15 s notices it within 2.5% of its life at a fraction of the
 * cost. The shared store polls at whatever its fastest consumer asked for, so
 * this never slows a flow run down.
 */
const BACKGROUND_POLL_INTERVAL_MS = 15_000;

export interface UseUnroutedApprovalsResult {
  /** Parks no other surface will show, oldest first (server order). */
  approvals: PendingApproval[];
  /** `request_id` currently being decided, or `null`. */
  decidingId: string | null;
  /** Set when the last decide failed; cleared on the next attempt. */
  error: string | null;
  decide: (requestId: string, decision: ApprovalDecisionInput) => Promise<void>;
}

export function useUnroutedApprovals(enabled = true): UseUnroutedApprovalsResult {
  const [decidingId, setDecidingId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const source = useFlowPendingApprovalsSource(enabled, BACKGROUND_POLL_INTERVAL_MS);
  const pendingApprovalByThread = useAppSelector(
    state => state.chatRuntime.pendingApprovalByThread
  );

  // Every request id a chat card has ever shown, not only the ones it shows
  // now. Deciding from the chat card clears its entry at once, while the
  // polled rows keep that park until the next poll (up to 15 s); matching on
  // "shown right now" would put the answered park back here as a second card
  // whose decide then fails with "already decided". A chat-owned park is
  // never unrouted, so once seen it stays excluded.
  const everChatRoutedIds = useRef(new Set<string>());
  const chatRoutedIds = useMemo(() => {
    for (const approval of Object.values(pendingApprovalByThread ?? {})) {
      if (approval?.requestId) everChatRoutedIds.current.add(approval.requestId);
    }
    return new Set(everChatRoutedIds.current);
  }, [pendingApprovalByThread]);

  const approvals = useMemo(
    () =>
      enabled
        ? source.approvals.filter(
            approval => !approval.source_context && !chatRoutedIds.has(approval.request_id)
          )
        : [],
    [chatRoutedIds, enabled, source.approvals]
  );

  const mountedRef = useRef(true);
  const decide = useCallback(async (requestId: string, decision: ApprovalDecisionInput) => {
    setDecidingId(requestId);
    setError(null);
    try {
      await decideApproval(requestId, decision);
      // Reconcile every approval surface at once rather than waiting out the
      // 15 s poll — the card must not linger after it has been answered.
      await refreshFlowPendingApprovals();
    } catch (err) {
      if (mountedRef.current) setError(err instanceof Error ? err.message : String(err));
      throw err;
    } finally {
      if (mountedRef.current) setDecidingId(null);
    }
  }, []);

  return { approvals, decidingId, error, decide };
}
